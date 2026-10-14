//! "Just built it, something's wrong" checks: problems a new or freshly set-up PC commonly has
//! that software can see (refresh rate, missing drivers, uninitialized drives, crash and
//! hardware-error logs, slow links, firmware security, activation, leftovers and startup bloat).
//! They only read data gathered at startup ([`crate::checkup`]) or live sensors.

use super::{Category, Context, Level, MIN_SAMPLES, Outcome, Step, guided};
use crate::bios_help::{self, BiosSetting, Board};
use crate::checkup::{
    Activation, Connection, CrashCause, EVENT_CAP, EVENT_DAYS, Event, MEMDIAG_DAYS, MEMDIAG_FAILED,
    Tpm, base_stop_code, crash_cause, format_link_speed, gpu_package_vendor, problem_text,
    stop_code_label, stop_code_name,
};
use crate::cpuid::Vendor;
use crate::format_bytes;
use crate::gpu::{VENDOR_AMD, VENDOR_INTEL, VENDOR_NVIDIA, pci_vendor_name};
use crate::storage::{PartitionStyle, sata_gen_speed};

const DDU_URL: &str = "https://www.wagnardsoft.com/display-driver-uninstaller-ddu-";
const MEMTEST_URL: &str = "https://www.memtest86.com/";
const MBR2GPT_URL: &str = "https://learn.microsoft.com/windows/deployment/mbr-to-gpt";

fn date(d: (i64, u32, u32)) -> String {
    format!("{}-{:02}-{:02}", d.0, d.1, d.2)
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// "200 or more" when a capped event list is full.
fn count_text(n: usize) -> String {
    if n >= EVENT_CAP {
        format!("{n} or more")
    } else {
        n.to_string()
    }
}

/// Official driver download page for a GPU vendor.
pub(super) fn driver_step(vendor: u32, what: &str) -> Step {
    match vendor {
        VENDOR_NVIDIA => Step::url(
            format!("Download {what} from NVIDIA (or install the NVIDIA App)."),
            "https://www.nvidia.com/en-us/drivers/",
        ),
        VENDOR_AMD => Step::url(
            format!("Download {what} from AMD (AMD Software: Adrenalin Edition)."),
            "https://www.amd.com/en/support/download/drivers.html",
        ),
        VENDOR_INTEL => Step::url(
            format!("Download {what} from Intel (Intel Driver & Support Assistant)."),
            "https://www.intel.com/content/www/us/en/support/detect.html",
        ),
        _ => Step::new(format!(
            "Download {what} from the graphics card or PC maker's website."
        )),
    }
}

/// Search step for the board vendor's driver downloads.
fn board_drivers_step(ctx: &Context, text: &str) -> Step {
    let name = Board::detect(ctx.info).search_name();
    let query = if name.is_empty() {
        "motherboard chipset drivers download".to_string()
    } else {
        format!("{name} drivers download")
    };
    Step::search(text, query)
}

fn memory_test_steps() -> Vec<Step> {
    vec![
        Step::new(
            "Test the memory: press Start, type \"Windows Memory Diagnostic\" and choose Restart now and check for problems.",
        ),
        Step::url(
            "For a more thorough test, run MemTest86 from a USB stick for a few passes.",
            MEMTEST_URL,
        ),
    ]
}

// ---- 1. Refresh rate ----------------------------------------------------------------------

pub(super) fn monitor_refresh(ctx: &Context) -> Outcome {
    let mons: Vec<_> = ctx
        .info
        .checkup
        .monitors
        .iter()
        .filter(|m| m.connection != Connection::Virtual)
        .collect();
    if mons.is_empty() {
        return Outcome::Skipped("No monitor information".into());
    }
    let mut out = Vec::new();
    for m in &mons {
        if !m.below_max() {
            continue;
        }
        let internal = m.connection == Connection::Internal;
        let mut steps = vec![Step::settings(
            format!(
                "Open Settings > System > Display > Advanced display, select {} and choose {} Hz under \"Choose a refresh rate\".",
                m.label(),
                m.max_hz
            ),
            "ms-settings:display-advanced",
        )];
        if !internal {
            steps.push(Step::new(
                "If the higher rate keeps resetting or flickers, use a DisplayPort cable (or an HDMI 2.0/2.1 cable rated for the resolution) plugged into the graphics card.",
            ));
        }
        steps.push(Step::new(
            "In games, also set the refresh rate (and any frame cap) to match.",
        ));
        out.push(guided(
            if internal { Level::Info } else { Level::Warning },
            Category::Gpu,
            format!(
                "{} runs at {:.0} Hz but supports {} Hz",
                m.label(),
                m.current_hz,
                m.max_hz
            ),
            format!(
                "Windows offers {} Hz for this monitor at {}x{}, but it's set to {:.0} Hz. A high-refresh monitor left at 60 Hz feels no smoother than a regular one.",
                m.max_hz, m.width, m.height, m.current_hz
            ),
            steps,
        ));
    }
    if out.is_empty() {
        return Outcome::Pass(
            mons.iter()
                .map(|m| format!("{}: {:.0} Hz", m.label(), m.current_hz))
                .collect::<Vec<_>>()
                .join("; "),
        );
    }
    Outcome::Findings(out)
}

// ---- 2. GPU driver --------------------------------------------------------------------------

/// Display adapters running without the vendor's driver.
pub(super) fn gpu_driver_missing(ctx: &Context) -> Outcome {
    let displays: Vec<_> = ctx
        .info
        .checkup
        .devices
        .iter()
        .filter(|d| d.is_display())
        .collect();
    if displays.is_empty() {
        return Outcome::Skipped("Display adapter list not available".into());
    }
    let mut out = Vec::new();
    for d in &displays {
        // Hyper-V and other virtual adapters use Microsoft drivers on purpose.
        let vendor = d.pci_vendor();
        let no_driver = d.is_basic_display() || d.problem.is_some_and(|p| matches!(p, 1 | 18 | 28));
        if !no_driver || vendor.is_none_or(|v| v == 0x1414) {
            continue;
        }
        let v = vendor.unwrap_or(0);
        let brand = pci_vendor_name(v).unwrap_or("graphics");
        out.push(guided(
            Level::Critical,
            Category::Gpu,
            format!("No {brand} graphics driver installed"),
            format!(
                "A {brand} graphics adapter is running on \"{}\" (Windows' fallback driver). Without the real driver, games run poorly or not at all, high refresh rates and resolutions are missing and the GPU can't be monitored.",
                d.name
            ),
            vec![
                driver_step(v, "the latest driver for your graphics card"),
                Step::new("Run the installer and restart when it asks."),
                Step::settings(
                    "If you can't download it, Windows Update often offers the driver under Advanced options > Optional updates.",
                    "ms-settings:windowsupdate-optionalupdates",
                ),
            ],
        ));
    }
    if out.is_empty() {
        return Outcome::Pass(
            displays
                .iter()
                .map(|d| d.name.as_str())
                .collect::<Vec<_>>()
                .join("; "),
        );
    }
    Outcome::Findings(out)
}

// ---- 3. Device Manager problems -------------------------------------------------------------

pub(super) fn device_problems(ctx: &Context) -> Outcome {
    let c = &ctx.info.checkup;
    if c.devices.is_empty() {
        return Outcome::Skipped("Device list not available".into());
    }
    // Display adapters without a driver are reported by the GPU driver check.
    let bad: Vec<_> = c
        .devices
        .iter()
        .filter(|d| d.problem.is_some() && !(d.is_display() && d.pci_vendor().is_some()))
        .collect();
    if bad.is_empty() {
        return Outcome::Pass("No devices with problems in Device Manager".into());
    }
    let list: Vec<String> = bad
        .iter()
        .take(8)
        .map(|d| {
            let p = d.problem.unwrap_or(0);
            format!("{}: {} (code {p})", d.name, problem_text(p))
        })
        .collect();
    let more = if bad.len() > 8 {
        format!(" and {} more", bad.len() - 8)
    } else {
        String::new()
    };
    let missing_driver = bad.iter().any(|d| matches!(d.problem, Some(1 | 18 | 28)));
    let mut steps = Vec::new();
    if missing_driver {
        steps.push(board_drivers_step(
            ctx,
            "Install the drivers from your motherboard or PC maker's support page: the chipset driver first, then LAN, Wi-Fi, Bluetooth and audio.",
        ));
        steps.push(Step::settings(
            "Also check Windows Update > Advanced options > Optional updates > Driver updates.",
            "ms-settings:windowsupdate-optionalupdates",
        ));
    }
    if bad.iter().any(|d| matches!(d.problem, Some(10 | 43))) {
        steps.push(Step::new(
            "For \"cannot start\" (code 10) or \"stopped\" (code 43) on USB devices: unplug and replug, try another port, and update the chipset driver.",
        ));
    }
    steps.push(Step::new(
        "Open Device Manager (right-click Start > Device Manager): devices with a yellow \"!\" are the ones listed here. Right-click > Update driver, or Uninstall device and restart to let Windows retry.",
    ));
    Outcome::Findings(vec![guided(
        Level::Warning,
        Category::System,
        format!(
            "{} not working properly",
            plural(bad.len(), "device is", "devices are")
        ),
        format!(
            "Device Manager reports: {}{more}. These are usually missing chipset, network or audio drivers on a fresh Windows install.",
            list.join("; ")
        ),
        steps,
    )])
}

// ---- 4. New drive not set up ----------------------------------------------------------------

pub(super) fn drive_setup(ctx: &Context) -> Outcome {
    let drives: Vec<_> = ctx
        .info
        .drives
        .iter()
        .filter(|d| d.size_bytes > 0 && d.layout.is_some())
        .collect();
    if drives.is_empty() {
        return Outcome::Skipped("Partition information not available".into());
    }
    let mut out = Vec::new();
    for d in &drives {
        let Some(l) = d.layout else { continue };
        let name = d.name();
        if l.style == PartitionStyle::Raw {
            out.push(guided(
                Level::Warning,
                Category::Storage,
                format!("{name} isn't set up yet"),
                format!(
                    "This {} drive has never been initialized, so Windows can't use it and it doesn't appear in File Explorer. This is normal for a brand-new drive.",
                    format_bytes(d.size_bytes)
                ),
                vec![
                    Step::settings(
                        "Open Settings > System > Storage > Advanced storage settings > Disks & volumes.",
                        "ms-settings:disksandvolumes",
                    ),
                    Step::new(format!(
                        "Select {name} and choose Initialize, with GPT as the partition style. Only do this for a new or empty drive."
                    )),
                    Step::new(
                        "Choose Create volume, pick a drive letter and format it as NTFS. It then shows up in File Explorer.",
                    ),
                ],
            ));
            continue;
        }
        let threshold = (d.size_bytes / 10).max(10 << 30);
        if l.unallocated_bytes >= threshold {
            let unused_drive = d.volumes.is_empty() && l.partitions <= 1;
            out.push(guided(
                if unused_drive { Level::Warning } else { Level::Info },
                Category::Storage,
                format!(
                    "{name} has {} of unallocated space",
                    format_bytes(l.unallocated_bytes)
                ),
                format!(
                    "{} of this {} drive isn't part of any volume, so it can't be used.",
                    format_bytes(l.unallocated_bytes),
                    format_bytes(d.size_bytes)
                ),
                vec![
                    Step::settings(
                        "Open Settings > System > Storage > Advanced storage settings > Disks & volumes.",
                        "ms-settings:disksandvolumes",
                    ),
                    Step::new(
                        "Select the unallocated space and choose Create volume (or extend the neighbouring volume in Disk Management).",
                    ),
                ],
            ));
        }
    }
    if out.is_empty() {
        return Outcome::Pass(format!(
            "{} set up",
            plural(drives.len(), "drive is", "drives are")
        ));
    }
    Outcome::Findings(out)
}

// ---- 5. Windows on a hard drive -------------------------------------------------------------

pub(super) fn system_drive_type(ctx: &Context) -> Outcome {
    let drives = &ctx.info.drives;
    let Some(sys) = drives
        .iter()
        .find(|d| d.volumes.iter().any(|v| v.mount.eq_ignore_ascii_case("C:")))
    else {
        return Outcome::Skipped("System drive not found".into());
    };
    match sys.ssd {
        None => Outcome::Skipped("Drive type not reported".into()),
        Some(true) => Outcome::Pass(format!("Windows is on {} ({})", sys.name(), sys.kind())),
        Some(false) => {
            let ssd = drives
                .iter()
                .find(|d| d.index != sys.index && d.ssd == Some(true) && !d.removable);
            let mut steps = Vec::new();
            let detail = match ssd {
                Some(s) => {
                    steps.push(Step::search(
                        format!(
                            "Move Windows to {}: clone the system drive with the SSD maker's migration tool, or do a clean install onto the SSD.",
                            s.name()
                        ),
                        format!("clone Windows to {}", s.name()),
                    ));
                    steps.extend(bios_help::bios_steps(
                        ctx.info,
                        BiosSetting::BootOrder,
                        "Make the SSD the first boot drive.",
                        "",
                    ));
                    format!(
                        "Windows runs from {}, a hard drive, while {} (an SSD) is installed. Hard drives start Windows and apps several times slower.",
                        sys.name(),
                        s.name()
                    )
                }
                None => {
                    steps.push(Step::new(
                        "Replace it with an SSD (even a small, inexpensive one makes Windows several times faster) and clone or reinstall Windows onto it.",
                    ));
                    format!(
                        "Windows runs from {}, a hard drive. Hard drives start Windows and apps several times slower than SSDs.",
                        sys.name()
                    )
                }
            };
            let mut f = guided(
                Level::Warning,
                Category::Storage,
                "Windows is installed on a hard drive",
                detail,
                steps,
            );
            f.bios = ssd.is_some();
            Outcome::Findings(vec![f])
        }
    }
}

// ---- 6. WHEA hardware errors ----------------------------------------------------------------

fn last_date(events: &[&Event]) -> String {
    events
        .iter()
        .map(|e| e.date)
        .max()
        .map(date)
        .unwrap_or_default()
}

fn stability_steps(ctx: &Context) -> Vec<Step> {
    let mut steps = bios_help::bios_steps(
        ctx.info,
        BiosSetting::LoadDefaults,
        "Load optimized defaults: this turns off XMP/EXPO, PBO/overclocks and undervolts.",
        "",
    );
    steps.push(Step::new(
        "Use the PC normally for a few days. If the errors stop, re-enable only XMP/EXPO and see whether they come back; if they do, try the next lower memory speed.",
    ));
    steps.extend(memory_test_steps());
    steps
}

pub(super) fn whea_errors(ctx: &Context) -> Outcome {
    let Some(ev) = &ctx.info.checkup.events else {
        return Outcome::Skipped("System event log not readable".into());
    };
    if ev.whea.is_empty() {
        return Outcome::Pass(format!(
            "No hardware errors logged in the last {EVENT_DAYS} days"
        ));
    }
    let by =
        |ids: &[u32]| -> Vec<&Event> { ev.whea.iter().filter(|e| ids.contains(&e.id)).collect() };
    let fatal = by(&[18]);
    let corrected_cpu_mem = by(&[19, 47]);
    let pcie = by(&[17]);
    let mut out = Vec::new();
    if !fatal.is_empty() || !corrected_cpu_mem.is_empty() {
        let all: Vec<&Event> = fatal.iter().chain(&corrected_cpu_mem).copied().collect();
        let n = all.len();
        out.push(
            guided(
                if fatal.is_empty() {
                    Level::Warning
                } else {
                    Level::Critical
                },
                Category::System,
                format!("{} hardware errors in the last {EVENT_DAYS} days", count_text(n)),
                format!(
                    "Windows logged {} CPU/memory hardware errors (WHEA-Logger{}), most recently on {}. On a new build this almost always means an unstable memory profile (XMP/EXPO), CPU overclock (PBO) or undervolt.",
                    count_text(n),
                    if fatal.is_empty() {
                        String::new()
                    } else {
                        format!(", {} of them fatal", fatal.len())
                    },
                    last_date(&all)
                ),
                stability_steps(ctx),
            )
            .bios(),
        );
    }
    if !pcie.is_empty() {
        out.push(guided(
            Level::Info,
            Category::System,
            format!("{} corrected PCIe errors in the last {EVENT_DAYS} days", count_text(pcie.len())),
            format!(
                "Windows corrected {} PCI Express errors (WHEA-Logger event 17), most recently on {}. They are fixed automatically, but frequent ones point at a device (often the graphics card or an NVMe drive) that isn't seated well or a link that's marginal.",
                count_text(pcie.len()),
                last_date(&pcie)
            ),
            vec![
                Step::new("Reseat the graphics card and M.2 drives, and check any riser cable."),
                board_drivers_step(ctx, "Update the motherboard BIOS and chipset driver."),
                Step::new(
                    "If the errors continue, setting the PCIe slot to the card's generation (instead of Auto) in the BIOS can help.",
                ),
            ],
        ));
    }
    Outcome::Findings(out)
}

// ---- 7. Unexpected shutdowns and blue screens -----------------------------------------------

pub(super) fn crashes(ctx: &Context) -> Outcome {
    let Some(ev) = &ctx.info.checkup.events else {
        return Outcome::Skipped("System event log not readable".into());
    };
    let hard: Vec<&Event> = ev
        .unexpected_shutdowns
        .iter()
        .filter(|e| e.code.is_none_or(|c| c == 0))
        .collect();
    if ev.bugchecks.is_empty() && hard.is_empty() {
        return Outcome::Pass(format!(
            "No blue screens or unexpected shutdowns in the last {EVENT_DAYS} days"
        ));
    }
    let mut out = Vec::new();
    if !ev.bugchecks.is_empty() {
        // Stop codes by frequency.
        let mut codes: Vec<(u32, usize)> = Vec::new();
        for e in &ev.bugchecks {
            let c = base_stop_code(e.code.unwrap_or(0));
            match codes.iter_mut().find(|(x, _)| *x == c) {
                Some((_, n)) => *n += 1,
                None => codes.push((c, 1)),
            }
        }
        codes.sort_by_key(|&(c, n)| (std::cmp::Reverse(n), c));
        let list = codes
            .iter()
            .map(|&(c, n)| {
                if n > 1 {
                    format!("{} x{n}", stop_code_label(c))
                } else {
                    stop_code_label(c)
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        let mut steps = Vec::new();
        let mut causes: Vec<CrashCause> = Vec::new();
        for &(c, _) in &codes {
            let cause = crash_cause(c);
            if !causes.contains(&cause) {
                causes.push(cause);
            }
        }
        let mut bios = false;
        for cause in causes {
            match cause {
                CrashCause::MemoryOrOverclock | CrashCause::Hardware => {
                    if !bios {
                        steps.push(Step::new(
                            "Memory and stability codes: undo memory and CPU tuning first.",
                        ));
                        steps.extend(stability_steps(ctx));
                        bios = true;
                    }
                }
                CrashCause::Graphics => {
                    steps.push(Step::url(
                        "Graphics driver crashes (VIDEO_TDR): clean-reinstall the GPU driver with Display Driver Uninstaller (DDU) in Safe Mode, then install the latest driver.",
                        DDU_URL,
                    ));
                    steps.push(Step::new(
                        "Check the graphics card's power cables are fully seated, and undo any GPU overclock.",
                    ));
                }
                CrashCause::Storage => steps.push(Step::new(
                    "Storage codes: check the drive health on the Storage tab, reseat or replace SATA cables, and update the SSD firmware.",
                )),
                CrashCause::Driver => steps.push(board_drivers_step(
                    ctx,
                    "Driver codes: update the chipset, network, audio and graphics drivers (board maker's support page first).",
                )),
                CrashCause::Unknown => {}
            }
        }
        if let Some(&(top, _)) = codes.first() {
            let q = match stop_code_name(top) {
                Some(n) => format!("{n} blue screen fix"),
                None => format!("stop code 0x{top:X} blue screen fix"),
            };
            steps.push(Step::search(
                format!(
                    "Look up the most frequent stop code, {}.",
                    stop_code_label(top)
                ),
                q,
            ));
        }
        let n = ev.bugchecks.len();
        let mut f = guided(
            if n >= 3 {
                Level::Critical
            } else {
                Level::Warning
            },
            Category::System,
            format!(
                "{} in the last {EVENT_DAYS} days",
                plural(n, "blue screen", "blue screens")
            ),
            format!(
                "Windows crashed with a blue screen {} times (most recently on {}). Stop codes: {list}.",
                count_text(n),
                last_date(&ev.bugchecks.iter().collect::<Vec<_>>())
            ),
            steps,
        );
        f.bios = bios;
        out.push(f);
    }
    if !hard.is_empty() {
        let n = hard.len();
        out.push(guided(
            if n >= 2 { Level::Warning } else { Level::Info },
            Category::System,
            format!(
                "{} without a blue screen",
                plural(n, "unexpected restart", "unexpected restarts")
            ),
            format!(
                "Windows restarted {} times without shutting down cleanly and without a blue screen (Kernel-Power 41), most recently on {}. Ignore any you caused by holding the power button or a power cut; otherwise the PC is losing power or freezing hard.",
                count_text(n),
                last_date(&hard)
            ),
            vec![
                Step::new(
                    "If it shuts off during games or heavy load, check the power supply: enough wattage for the GPU, and separate PCIe cables for each GPU power connector.",
                ),
                Step::new(
                    "Watch CPU and GPU temperatures on the Sensors tab under load: a PC that switches itself off can be overheating.",
                ),
                Step::new("Undo overclocks and undervolts (CPU, GPU and memory) and see whether it stops."),
            ],
        ));
    }
    Outcome::Findings(out)
}

// ---- 8. Memory diagnostic result ------------------------------------------------------------

pub(super) fn memory_diagnostic(ctx: &Context) -> Outcome {
    let Some(ev) = &ctx.info.checkup.events else {
        return Outcome::Skipped("System event log not readable".into());
    };
    let Some(e) = &ev.memory_diagnostic else {
        return Outcome::Skipped(format!(
            "No Windows Memory Diagnostic run in the last {MEMDIAG_DAYS} days"
        ));
    };
    if !MEMDIAG_FAILED.contains(&e.id) {
        return Outcome::Pass(format!("Last test passed ({})", date(e.date)));
    }
    let mut steps = vec![
        Step::new(
            "If memory overclocking (XMP/EXPO) is on, turn it off and test again: an unstable profile also shows up as errors.",
        ),
        Step::new(
            "Test one module at a time (power off before swapping) to find the bad one, then replace it under warranty.",
        ),
    ];
    steps.extend(memory_test_steps().into_iter().skip(1));
    Outcome::Findings(vec![guided(
        Level::Critical,
        Category::Memory,
        "Windows Memory Diagnostic found errors",
        format!(
            "The memory test on {} reported hardware errors. Faulty or unstable memory causes crashes, corrupted files and failed installs.",
            date(e.date)
        ),
        steps,
    )
    .bios()])
}

// ---- 9. NVIDIA hardware slowdown ------------------------------------------------------------

pub(super) fn gpu_hw_slowdown(ctx: &Context) -> Outcome {
    let mut out = Vec::new();
    let mut seen = Vec::new();
    let mut waiting = false;
    for g in ctx.info.gpus.iter().filter(|g| g.nvml.is_some()) {
        let pre = g.sensor_prefix();
        let get = |id: &str| ctx.sensors.get(&format!("{pre}/{id}"));
        let (Some(load), Some(slow), Some(thermal), Some(brake)) = (
            get("nv/load"),
            get("nv/throttle/hw_slowdown"),
            get("nv/throttle/hw_thermal"),
            get("nv/throttle/power_brake"),
        ) else {
            continue;
        };
        // All histories are appended on every sample, so align them from the newest end.
        let busy: Vec<(f32, f32, f32)> = load
            .history
            .iter()
            .rev()
            .zip(slow.history.iter().rev())
            .zip(thermal.history.iter().rev())
            .zip(brake.history.iter().rev())
            .filter(|(((l, _), _), _)| **l >= 50.0)
            .map(|(((_, s), t), b)| (*s, *t, *b))
            .collect();
        if busy.len() < MIN_SAMPLES {
            waiting = true;
            continue;
        }
        let count = |f: fn(&(f32, f32, f32)) -> f32| busy.iter().filter(|x| f(x) >= 0.5).count();
        let (n_slow, n_thermal, n_brake) = (count(|x| x.0), count(|x| x.1), count(|x| x.2));
        seen.push(format!("{}: no hardware slowdown under load", g.name));
        let enough = (busy.len() / 20).max(3);
        let power_steps = || {
            vec![
                Step::new(
                    "Power the card with separate PCIe cables from the power supply, one per 8-pin socket; avoid a single cable with a daisy-chained second plug.",
                ),
                Step::new(
                    "On 12VHPWR / 12V-2x6 cards, push the connector in fully until it clicks, with no sharp bend right at the plug.",
                ),
                Step::new(
                    "Check the power supply meets the card maker's recommended wattage, and that it isn't old or failing.",
                ),
            ]
        };
        if n_brake >= enough || (n_slow >= enough && n_thermal < enough) {
            seen.pop();
            out.push(guided(
                Level::Warning,
                Category::Gpu,
                format!("{} is held back by its power input", g.name),
                format!(
                    "Under load the card signalled a hardware {} in {} of {} samples. The GPU drops its clocks sharply when its power input looks unsafe, usually because of a loose or daisy-chained power cable or a weak power supply.",
                    if n_brake >= enough { "power brake" } else { "slowdown" },
                    n_brake.max(n_slow),
                    busy.len()
                ),
                power_steps(),
            ));
        } else if n_thermal >= enough {
            seen.pop();
            out.push(guided(
                Level::Warning,
                Category::Gpu,
                format!("{} hits its hardware temperature limit", g.name),
                format!(
                    "Under load the card triggered its hardware thermal slowdown in {n_thermal} of {} samples, cutting clocks hard to protect itself.",
                    busy.len()
                ),
                vec![
                    Step::new("Clean dust from the card's heatsink and fans and the case filters."),
                    Step::new("Improve case airflow and set a more aggressive fan curve."),
                    Step::new(
                        "On older cards, new thermal pads and paste often fix hot memory or hotspot temperatures.",
                    ),
                ],
            ));
        }
    }
    if !out.is_empty() {
        return Outcome::Findings(out);
    }
    if !seen.is_empty() {
        return Outcome::Pass(seen.join("; "));
    }
    Outcome::Skipped(if waiting {
        "Needs some time with the GPU under load (e.g. a game) to judge".into()
    } else {
        "Needs an NVIDIA GPU (throttle reasons via NVML)".into()
    })
}

// ---- 10. Ethernet link speed ----------------------------------------------------------------

pub(super) fn ethernet_speed(ctx: &Context) -> Outcome {
    let nics = &ctx.info.checkup.nics;
    if nics.is_empty() {
        return Outcome::Skipped("No wired network connection".into());
    }
    let mut out = Vec::new();
    for n in nics {
        if n.speed_bps > 100_000_000 {
            continue;
        }
        out.push(guided(
            Level::Warning,
            Category::System,
            format!("Ethernet connected at only {}", format_link_speed(n.speed_bps)),
            format!(
                "{} linked at {}. Gigabit (1 Gbps) is standard on PCs and routers; 100 Mbps almost always means a damaged cable, a cable with only four wires, or a bad port, and caps downloads at about 12 MB/s.",
                n.name,
                format_link_speed(n.speed_bps)
            ),
            vec![
                Step::new("Replace the network cable with a Cat5e or Cat6 one, and make sure both plugs click in."),
                Step::new("Try a different port on the router or switch, and check that it supports gigabit."),
                Step::search(
                    "Update the network adapter's driver.",
                    format!("{} driver download", n.name),
                ),
            ],
        ));
    }
    if out.is_empty() {
        return Outcome::Pass(
            nics.iter()
                .map(|n| format!("{}: {}", n.name, format_link_speed(n.speed_bps)))
                .collect::<Vec<_>>()
                .join("; "),
        );
    }
    Outcome::Findings(out)
}

// ---- 11. NVMe / SATA link -------------------------------------------------------------------

pub(super) fn storage_link(ctx: &Context) -> Outcome {
    let board = Board::detect(ctx.info);
    let manual_query = |what: &str| {
        let name = board.search_name();
        if name.is_empty() {
            format!("motherboard manual {what}")
        } else {
            format!("{name} manual {what}")
        }
    };
    let mut out = Vec::new();
    let mut seen = Vec::new();
    for l in &ctx.info.checkup.nvme_links {
        let ((cg, cw), (mg, mw)) = (l.current, l.max);
        if cg == 0 || cw == 0 || mg == 0 || mw == 0 {
            continue;
        }
        seen.push(format!("{}: PCIe {cg}.0 x{cw}", l.device));
        let slot_steps = |what: &str| {
            vec![
                Step::search(
                    format!(
                        "Check the motherboard manual for which M.2 slot is wired to the CPU at full speed (usually the top one) and move the drive there{what}."
                    ),
                    manual_query("M.2 slot PCIe lanes"),
                ),
                Step::new(
                    "Some M.2 slots share lanes with SATA ports or a PCIe slot; the manual lists what disables or slows what.",
                ),
            ]
        };
        if cw < mw {
            out.push(guided(
                Level::Warning,
                Category::Storage,
                format!("{} runs at PCIe x{cw} instead of x{mw}", l.device),
                format!(
                    "The drive supports {mw} PCIe lanes but its link uses {cw}, roughly {} its maximum speed. The M.2 slot is probably wired with fewer lanes or shares them with another device.",
                    if cw * 2 <= mw { "halving" } else { "reducing" }
                ),
                slot_steps(""),
            ));
        } else if cg < mg {
            out.push(guided(
                Level::Info,
                Category::Storage,
                format!("{} runs at PCIe {cg}.0 (supports {mg}.0)", l.device),
                format!(
                    "The drive supports PCIe {mg}.0 but the link runs at {cg}.0, which caps sequential speed. Common when the drive sits in a slot wired to the chipset or an older CPU generation."
                ),
                slot_steps(" if you want the full speed"),
            ));
        }
    }
    for d in &ctx.info.drives {
        let Some(s) = d.sata_link else { continue };
        seen.push(format!(
            "{}: SATA {}",
            d.name(),
            sata_gen_speed(s.current_gen)
        ));
        if s.current_gen < s.max_gen {
            out.push(guided(
                Level::Warning,
                Category::Storage,
                format!(
                    "{} links at {} instead of {}",
                    d.name(),
                    sata_gen_speed(s.current_gen),
                    sata_gen_speed(s.max_gen)
                ),
                format!(
                    "The drive supports SATA {} but negotiated {}. For an SSD that halves its speed or worse.",
                    sata_gen_speed(s.max_gen),
                    sata_gen_speed(s.current_gen)
                ),
                vec![
                    Step::search(
                        "Connect it to a SATA 6 Gb/s port: some boards also have slower ports or ports on an extra controller (see the manual).",
                        manual_query("SATA ports"),
                    ),
                    Step::new("Replace the SATA data cable (use one rated SATA III / 6 Gb/s)."),
                ],
            ));
        }
    }
    if !out.is_empty() {
        return Outcome::Findings(out);
    }
    if seen.is_empty() {
        return Outcome::Skipped(
            "No NVMe link data, and SATA link speed needs administrator rights".into(),
        );
    }
    Outcome::Pass(seen.join("; "))
}

// ---- 12. Secure Boot / TPM / legacy boot ----------------------------------------------------

pub(super) fn firmware_security(ctx: &Context) -> Outcome {
    let fw = &ctx.info.checkup.firmware;
    if fw.uefi.is_none() && fw.secure_boot.is_none() && fw.tpm == Tpm::Unknown {
        return Outcome::Skipped("Firmware information not available".into());
    }
    let why = "Windows 11 and the anti-cheat in games such as Valorant, Battlefield 6 and FACEIT need it.";
    let mut out = Vec::new();
    if fw.uefi == Some(false) {
        let mut steps = vec![Step::url(
            "Convert the Windows disk from MBR to GPT without losing data: in an administrator terminal run \"mbr2gpt /validate /allowFullOS\", then \"mbr2gpt /convert /allowFullOS\". Back up first.",
            MBR2GPT_URL,
        )];
        steps.extend(bios_help::bios_steps(
            ctx.info,
            BiosSetting::Csm,
            "Turn off CSM / legacy boot so the PC boots in UEFI mode.",
            "",
        ));
        out.push(
            guided(
                Level::Warning,
                Category::System,
                "Windows boots in legacy BIOS (CSM) mode",
                format!(
                    "The PC starts Windows through the old BIOS compatibility mode, which rules out Secure Boot and Resizable BAR. {why}"
                ),
                steps,
            )
            .security(),
        );
    } else if fw.secure_boot == Some(false) {
        out.push(
            guided(
                Level::Warning,
                Category::System,
                "Secure Boot is off",
                format!("Secure Boot checks that only trusted code runs while the PC starts. {why}"),
                bios_help::bios_steps(
                    ctx.info,
                    BiosSetting::SecureBoot,
                    "Turn on Secure Boot (some boards first need the default keys installed, or CSM turned off).",
                    "",
                ),
            )
            .security()
            .bios(),
        );
    }
    match fw.tpm {
        Tpm::Missing => out.push(
            guided(
                Level::Warning,
                Category::System,
                "No TPM found",
                format!(
                    "The PC has no active TPM (Trusted Platform Module), used for BitLocker, Windows Hello and secure sign-in. {why} Most CPUs since 2017 have a firmware TPM that's simply turned off."
                ),
                bios_help::bios_steps(
                    ctx.info,
                    BiosSetting::Tpm,
                    "Turn on the firmware TPM (AMD fTPM or Intel PTT).",
                    "",
                ),
            )
            .security()
            .bios(),
        ),
        Tpm::V12 => out.push(guided(
            Level::Info,
            Category::System,
            "Only TPM 1.2 is available",
            "Windows 11 needs TPM 2.0. Many boards can switch the firmware TPM to 2.0, or a discrete TPM 2.0 module can be added.",
            vec![Step::new(
                "Check the BIOS security settings for a TPM 2.0 / firmware TPM option.",
            )],
        )
        .security()
        .bios()),
        _ => {}
    }
    if out.is_empty() {
        let mut parts = Vec::new();
        if fw.uefi == Some(true) {
            parts.push("UEFI boot");
        }
        if fw.secure_boot == Some(true) {
            parts.push("Secure Boot on");
        }
        if fw.tpm == Tpm::V20 {
            parts.push("TPM 2.0");
        }
        return Outcome::Pass(parts.join(", "));
    }
    Outcome::Findings(out)
}

// ---- 13. Activation -------------------------------------------------------------------------

pub(super) fn activation(ctx: &Context) -> Outcome {
    match ctx.info.checkup.activation {
        None => Outcome::Skipped("Activation status not available".into()),
        Some(Activation::Activated) => Outcome::Pass("Windows is activated".into()),
        Some(state) => Outcome::Findings(vec![guided(
            Level::Warning,
            Category::System,
            if state == Activation::Tampered {
                "Windows license isn't genuine"
            } else {
                "Windows isn't activated"
            },
            "An unactivated Windows shows a watermark, locks personalization settings and keeps reminding you.",
            vec![
                Step::settings(
                    "Open Settings > System > Activation and enter your product key, or buy a license there.",
                    "ms-settings:activation",
                ),
                Step::new(
                    "If Windows was activated before you changed the motherboard, run the Troubleshoot option there and choose \"I changed hardware on this device recently\" (sign in with the Microsoft account the license is linked to).",
                ),
            ],
        )]),
    }
}

// ---- 14. Leftover GPU drivers ---------------------------------------------------------------

pub(super) fn leftover_gpu_drivers(ctx: &Context) -> Outcome {
    let packages = &ctx.info.checkup.gpu_driver_packages;
    let mut present: Vec<u32> = ctx.info.gpus.iter().map(|g| g.vendor_id).collect();
    present.extend(
        ctx.info
            .checkup
            .devices
            .iter()
            .filter(|d| d.is_display())
            .filter_map(|d| d.pci_vendor()),
    );
    let intel_cpu = ctx.info.cpu.vendor == Vendor::Intel;
    let leftovers: Vec<(&String, u32)> = packages
        .iter()
        .filter_map(|p| Some((p, gpu_package_vendor(p)?)))
        // An Intel CPU's graphics may just be switched off in the BIOS.
        .filter(|&(_, v)| !present.contains(&v) && !(v == VENDOR_INTEL && intel_cpu))
        .collect();
    if leftovers.is_empty() {
        return Outcome::Pass(if packages.is_empty() {
            "No graphics driver packages listed".into()
        } else {
            "Installed graphics drivers match the GPUs".into()
        });
    }
    let names: Vec<&str> = leftovers.iter().map(|(p, _)| p.as_str()).collect();
    let mut vendors: Vec<&str> = leftovers
        .iter()
        .filter_map(|&(_, v)| pci_vendor_name(v))
        .collect();
    vendors.dedup();
    Outcome::Findings(vec![guided(
        Level::Info,
        Category::Gpu,
        format!(
            "Old {} graphics driver still installed",
            vendors.join(" and ")
        ),
        format!(
            "{} is installed, but no such graphics card is present. Leftovers from a previous card can cause crashes, black screens or the wrong control panel opening.",
            names.join(", ")
        ),
        vec![
            Step::settings(
                format!(
                    "Uninstall {} in Settings > Apps > Installed apps.",
                    names.join(" and ")
                ),
                "ms-settings:appsfeatures",
            ),
            Step::url(
                "For a thorough clean-up (recommended after switching between NVIDIA and AMD), use Display Driver Uninstaller (DDU) in Safe Mode, then reinstall the current card's driver.",
                DDU_URL,
            ),
        ],
    )])
}

// ---- 15. Startup apps -----------------------------------------------------------------------

/// More enabled startup apps than this is flagged.
const STARTUP_LIMIT: usize = 15;

pub(super) fn startup_apps(ctx: &Context) -> Outcome {
    let Some(apps) = &ctx.info.checkup.startup_apps else {
        return Outcome::Skipped("Startup apps not readable".into());
    };
    let n = apps.len();
    if n <= STARTUP_LIMIT {
        return Outcome::Pass(plural(n, "startup app", "startup apps"));
    }
    let shown: Vec<&str> = apps.iter().take(12).map(|s| s.as_str()).collect();
    Outcome::Findings(vec![guided(
        Level::Info,
        Category::System,
        format!("{n} apps start with Windows"),
        format!(
            "{n} apps launch at sign-in ({}{}). Each adds to startup time and keeps running in the background.",
            shown.join(", "),
            if n > shown.len() { ", ..." } else { "" }
        ),
        vec![
            Step::settings(
                "Open Settings > Apps > Startup and turn off the apps you don't need right away (launchers, updaters, chat apps).",
                "ms-settings:startupapps",
            ),
            Step::new(
                "Task Manager's Startup apps page shows the same list with each app's startup impact.",
            ),
        ],
    )])
}

#[cfg(test)]
mod tests {
    use super::super::tests::{ctx, levels};
    use super::*;
    use crate::SystemInfo;
    use crate::checkup::{Device, Events, Monitor, NicLink, PcieLink};
    use crate::sensors::{Reading, SensorTable, Unit};
    use crate::storage::{DiskLayout, Drive, SataLink, Volume};

    fn ev(id: u32, code: Option<u32>) -> Event {
        Event {
            id,
            date: (2026, 10, 1),
            code,
        }
    }

    #[test]
    fn refresh_rate() {
        let s = SensorTable::default();
        let mut info = SystemInfo::default();
        assert!(matches!(
            monitor_refresh(&ctx(&info, &s)),
            Outcome::Skipped(_)
        ));
        info.checkup.monitors = vec![Monitor {
            name: "Gaming".into(),
            connection: Connection::Hdmi,
            width: 2560,
            height: 1440,
            current_hz: 60.0,
            max_hz: 144,
            ..Default::default()
        }];
        let o = monitor_refresh(&ctx(&info, &s));
        assert_eq!(levels(&o), vec![Level::Warning]);
        let Outcome::Findings(f) = o else {
            unreachable!()
        };
        assert_eq!(
            f[0].steps[0].link,
            Some(super::super::Link::Settings(
                "ms-settings:display-advanced".into()
            ))
        );
        info.checkup.monitors[0].current_hz = 143.97;
        assert!(matches!(monitor_refresh(&ctx(&info, &s)), Outcome::Pass(_)));
    }

    #[test]
    fn drivers_and_devices() {
        let s = SensorTable::default();
        let dev = |name: &str, class: &str, service: &str, hw: &str, problem| Device {
            name: name.into(),
            class: class.into(),
            service: service.into(),
            hardware_id: hw.into(),
            problem,
        };
        let mut info = SystemInfo::default();
        info.checkup.devices = vec![
            dev(
                "Microsoft Basic Display Adapter",
                "Display",
                "BasicDisplay",
                r"PCI\VEN_10DE&DEV_2204",
                None,
            ),
            dev(
                "Microsoft Hyper-V Video",
                "Display",
                "BasicDisplay",
                r"VMBUS\{x}",
                None,
            ),
        ];
        let o = gpu_driver_missing(&ctx(&info, &s));
        assert_eq!(levels(&o), vec![Level::Critical]);
        let Outcome::Findings(f) = o else {
            unreachable!()
        };
        assert!(f[0].title.contains("NVIDIA"));
        assert!(matches!(device_problems(&ctx(&info, &s)), Outcome::Pass(_)));

        info.checkup.devices.push(dev(
            "Ethernet Controller",
            "Other",
            "",
            r"PCI\VEN_8086&DEV_15F3",
            Some(28),
        ));
        let o = device_problems(&ctx(&info, &s));
        assert_eq!(levels(&o), vec![Level::Warning]);
        let Outcome::Findings(f) = o else {
            unreachable!()
        };
        assert!(
            f[0].detail
                .contains("Ethernet Controller: no driver installed (code 28)")
        );
        assert!(matches!(
            f[0].steps[0].link,
            Some(super::super::Link::Search(_))
        ));
    }

    #[test]
    fn drive_setup_and_type() {
        const G: u64 = 1 << 30;
        let s = SensorTable::default();
        let mut info = SystemInfo {
            drives: vec![
                Drive {
                    index: 0,
                    model: "Big HDD".into(),
                    size_bytes: 2000 * G,
                    ssd: Some(false),
                    volumes: vec![Volume {
                        mount: "C:".into(),
                        ..Default::default()
                    }],
                    layout: Some(DiskLayout {
                        style: PartitionStyle::Gpt,
                        partitions: 3,
                        unallocated_bytes: G / 100,
                    }),
                    ..Default::default()
                },
                Drive {
                    index: 1,
                    model: "New SSD".into(),
                    size_bytes: 1000 * G,
                    ssd: Some(true),
                    layout: Some(DiskLayout {
                        style: PartitionStyle::Raw,
                        partitions: 0,
                        unallocated_bytes: 1000 * G,
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let c = ctx(&info, &s);
        let o = drive_setup(&c);
        assert_eq!(levels(&o), vec![Level::Warning]);
        let Outcome::Findings(f) = o else {
            unreachable!()
        };
        assert!(f[0].title.contains("New SSD"));
        let o = system_drive_type(&c);
        assert_eq!(levels(&o), vec![Level::Warning]);
        let Outcome::Findings(f) = o else {
            unreachable!()
        };
        assert!(f[0].bios && f[0].detail.contains("New SSD"));

        info.drives[1].layout = Some(DiskLayout {
            style: PartitionStyle::Gpt,
            partitions: 2,
            unallocated_bytes: 500 * G,
        });
        info.drives[0].ssd = Some(true);
        let c = ctx(&info, &s);
        assert_eq!(levels(&drive_setup(&c)), vec![Level::Info]);
        assert!(matches!(system_drive_type(&c), Outcome::Pass(_)));
    }

    #[test]
    fn event_checks() {
        let s = SensorTable::default();
        let mut info = SystemInfo::default();
        assert!(matches!(whea_errors(&ctx(&info, &s)), Outcome::Skipped(_)));
        info.checkup.events = Some(Events::default());
        let c = ctx(&info, &s);
        assert!(matches!(whea_errors(&c), Outcome::Pass(_)));
        assert!(matches!(crashes(&c), Outcome::Pass(_)));
        assert!(matches!(memory_diagnostic(&c), Outcome::Skipped(_)));

        info.checkup.events = Some(Events {
            whea: vec![ev(19, None), ev(19, None), ev(17, None)],
            unexpected_shutdowns: vec![ev(41, Some(0)), ev(41, Some(0x1A)), ev(41, None)],
            bugchecks: vec![ev(1001, Some(0x1A)), ev(1001, Some(0x116))],
            memory_diagnostic: Some(ev(1102, None)),
        });
        let c = ctx(&info, &s);
        let o = whea_errors(&c);
        assert_eq!(levels(&o), vec![Level::Warning, Level::Info]);
        let Outcome::Findings(f) = o else {
            unreachable!()
        };
        assert!(f[0].bios);
        let o = crashes(&c);
        assert_eq!(levels(&o), vec![Level::Warning, Level::Warning]);
        let Outcome::Findings(f) = o else {
            unreachable!()
        };
        assert!(f[0].detail.contains("MEMORY_MANAGEMENT (0x1A)"));
        assert!(f[0].steps.iter().any(|s| s.text.contains("DDU")));
        assert!(f[1].title.starts_with("2 unexpected restarts"));
        assert_eq!(levels(&memory_diagnostic(&c)), vec![Level::Critical]);
    }

    #[test]
    fn gpu_slowdown() {
        use crate::gpu::{GpuAdapter, NvmlDetails};
        let info = SystemInfo {
            gpus: vec![GpuAdapter {
                name: "RTX".into(),
                vendor_id: VENDOR_NVIDIA,
                luid: 1,
                nvml: Some(NvmlDetails::default()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut s = SensorTable::default();
        let feed = |s: &mut SensorTable, load: f64, slow: f64, brake: f64| {
            let r = |id: &str, v: f64| Reading {
                id: format!("gpu/1/{id}"),
                label: id.into(),
                group: "g".into(),
                unit: Unit::None,
                value: v,
            };
            s.update([
                r("nv/load", load),
                r("nv/throttle/hw_slowdown", slow),
                r("nv/throttle/hw_thermal", 0.0),
                r("nv/throttle/power_brake", brake),
            ]);
        };
        assert!(matches!(
            gpu_hw_slowdown(&ctx(&info, &s)),
            Outcome::Skipped(_)
        ));
        for _ in 0..20 {
            feed(&mut s, 99.0, 0.0, 0.0);
        }
        assert!(matches!(gpu_hw_slowdown(&ctx(&info, &s)), Outcome::Pass(_)));
        for _ in 0..10 {
            feed(&mut s, 99.0, 1.0, 1.0);
        }
        assert_eq!(
            levels(&gpu_hw_slowdown(&ctx(&info, &s))),
            vec![Level::Warning]
        );
    }

    #[test]
    fn links_and_network() {
        let s = SensorTable::default();
        let mut info = SystemInfo::default();
        assert!(matches!(
            ethernet_speed(&ctx(&info, &s)),
            Outcome::Skipped(_)
        ));
        assert!(matches!(storage_link(&ctx(&info, &s)), Outcome::Skipped(_)));
        info.checkup.nics = vec![NicLink {
            name: "Realtek PCIe 2.5GbE".into(),
            speed_bps: 100_000_000,
        }];
        assert_eq!(
            levels(&ethernet_speed(&ctx(&info, &s))),
            vec![Level::Warning]
        );
        info.checkup.nics[0].speed_bps = 2_500_000_000;
        assert!(matches!(ethernet_speed(&ctx(&info, &s)), Outcome::Pass(_)));

        info.checkup.nvme_links = vec![
            PcieLink {
                device: "Fast SSD".into(),
                current: (4, 2),
                max: (4, 4),
            },
            PcieLink {
                device: "Other SSD".into(),
                current: (3, 4),
                max: (4, 4),
            },
        ];
        info.drives.push(Drive {
            model: "SATA SSD".into(),
            sata_link: Some(SataLink {
                current_gen: 2,
                max_gen: 3,
            }),
            ..Default::default()
        });
        assert_eq!(
            levels(&storage_link(&ctx(&info, &s))),
            vec![Level::Warning, Level::Info, Level::Warning]
        );
    }

    #[test]
    fn firmware_and_software() {
        use crate::checkup::Firmware;
        let s = SensorTable::default();
        let mut info = SystemInfo::default();
        assert!(matches!(
            firmware_security(&ctx(&info, &s)),
            Outcome::Skipped(_)
        ));
        info.checkup.firmware = Firmware {
            uefi: Some(true),
            secure_boot: Some(false),
            tpm: Tpm::Missing,
        };
        let o = firmware_security(&ctx(&info, &s));
        assert_eq!(levels(&o), vec![Level::Warning, Level::Warning]);
        let Outcome::Findings(f) = o else {
            unreachable!()
        };
        assert!(f.iter().all(|f| f.security && f.bios));
        info.checkup.firmware = Firmware {
            uefi: Some(false),
            secure_boot: Some(false),
            tpm: Tpm::V20,
        };
        let o = firmware_security(&ctx(&info, &s));
        assert_eq!(levels(&o), vec![Level::Warning]);
        info.checkup.firmware.uefi = Some(true);
        info.checkup.firmware.secure_boot = Some(true);
        assert!(matches!(
            firmware_security(&ctx(&info, &s)),
            Outcome::Pass(_)
        ));

        assert!(matches!(activation(&ctx(&info, &s)), Outcome::Skipped(_)));
        info.checkup.activation = Some(Activation::NotActivated);
        assert_eq!(levels(&activation(&ctx(&info, &s))), vec![Level::Warning]);

        info.checkup.gpu_driver_packages = vec![
            "AMD Software".into(),
            "NVIDIA Graphics Driver 581.57".into(),
        ];
        info.gpus = vec![crate::gpu::GpuAdapter {
            vendor_id: VENDOR_NVIDIA,
            ..Default::default()
        }];
        let o = leftover_gpu_drivers(&ctx(&info, &s));
        assert_eq!(levels(&o), vec![Level::Info]);
        let Outcome::Findings(f) = o else {
            unreachable!()
        };
        assert!(f[0].title.contains("AMD") && !f[0].detail.contains("NVIDIA"));

        info.checkup.startup_apps = Some((0..20).map(|i| format!("App{i}")).collect());
        assert_eq!(levels(&startup_apps(&ctx(&info, &s))), vec![Level::Info]);
        info.checkup.startup_apps = Some(vec!["OneDrive".into()]);
        assert!(matches!(startup_apps(&ctx(&info, &s)), Outcome::Pass(_)));
    }
}
