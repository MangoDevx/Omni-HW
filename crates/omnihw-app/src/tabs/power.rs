//! Power page: power profile, CPU frequency scaling, the firmware platform profile, power
//! saving and sleep. Read on a worker thread when the page opens (see [`Sampler::read_power`]);
//! changes run on worker threads too, kernel settings through the pkexec helper.

use super::{group_box, or_dash};
use crate::heat::{GREEN, ORANGE, ink};
use crate::sampler::Sampler;
use egui::{Button, RichText, Ui};
use omnihw_core::linux::power::{Choice, PowerInfo, Setting};

/// What a governor does, for its hover text.
fn governor_hint(name: &str, driver: &str) -> &'static str {
    let pstate = driver.starts_with("intel_pstate") || driver.starts_with("amd-pstate");
    match name {
        "performance" => "Keeps the CPU at high clocks. Fastest response, more power and heat.",
        "powersave" if pstate => {
            "The CPU picks its clocks itself, guided by the energy/performance preference. The usual setting with this driver."
        }
        "powersave" => "Holds the CPU at its lowest clocks. Slow: only for saving battery.",
        "schedutil" => "Follows the scheduler's load. The usual default.",
        "ondemand" => "Raises clocks quickly under load.",
        "conservative" => "Raises clocks gradually under load.",
        "userspace" => "Clocks set by a program. Rarely used.",
        _ => "",
    }
}

fn epp_hint(name: &str) -> &'static str {
    match name {
        "performance" => "Favors speed over power use.",
        "balance_performance" => "Leans towards speed. The usual default on desktops.",
        "default" => "The firmware's default.",
        "balance_power" => "Leans towards saving power.",
        "power" => "Favors saving power over speed.",
        _ => "",
    }
}

/// A row with a drop-down of `choice`'s options; returns the option picked this frame.
fn choice_row(
    ui: &mut Ui,
    label: &str,
    choice: &Choice,
    enabled: bool,
    hint: impl Fn(&str) -> &'static str,
) -> Option<String> {
    let mut picked = None;
    ui.label(label);
    let current = if choice.current.is_empty() {
        "mixed"
    } else {
        &choice.current
    };
    ui.add_enabled_ui(enabled, |ui| {
        egui::ComboBox::from_id_salt(label)
            .selected_text(current)
            .show_ui(ui, |ui| {
                for o in &choice.options {
                    let r = ui.selectable_label(*o == choice.current, o);
                    let r = match hint(o) {
                        "" => r,
                        h => r.on_hover_text(h),
                    };
                    if r.clicked() && *o != choice.current {
                        picked = Some(o.clone());
                    }
                }
            });
    });
    ui.end_row();
    picked
}

fn value_row(ui: &mut Ui, label: &str, value: &str) {
    ui.label(label);
    ui.add(egui::Label::new(RichText::new(or_dash(value)).strong()).wrap());
    ui.end_row();
}

fn aspm_text(policy: &str) -> &'static str {
    match policy {
        "default" => "default: as the firmware set it up",
        "performance" => "performance: link power saving off",
        "powersave" => "powersave: link power saving on",
        "powersupersave" => "powersupersave: deepest link power saving",
        _ => "",
    }
}

fn sleep_text(mode: &str) -> String {
    match mode {
        "deep" => "deep (suspend to RAM, S3)".into(),
        "s2idle" => "s2idle (suspend to idle; the CPU stays in a light sleep)".into(),
        "shallow" => "shallow (standby)".into(),
        m => m.into(),
    }
}

pub fn show(ui: &mut Ui, sampler: &Sampler) {
    // Clones under the lock, released before drawing.
    let (info, busy, msg) = {
        let snap = sampler.snapshot();
        (snap.power.clone(), snap.power_busy, snap.power_msg.clone())
    };
    super::page_title(ui, "Power");
    ui.weak(
        "Power profile, CPU frequency scaling, power saving and sleep. Kernel settings ask for \
         your password and last until the next restart.",
    );
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!busy, Button::new("Refresh"))
            .on_hover_text("Read the power settings again")
            .clicked()
        {
            sampler.read_power();
        }
        if busy {
            ui.spinner();
        }
        match &msg {
            Some(Ok(m)) => {
                ui.colored_label(ink(ui.visuals(), GREEN), m);
            }
            Some(Err(e)) => {
                ui.colored_label(ink(ui.visuals(), ORANGE), e);
            }
            None => {}
        }
    });
    ui.add_space(6.0);
    let Some(p) = info else {
        return;
    };
    profile_section(ui, sampler, &p, busy);
    cpu_section(ui, sampler, &p, busy);
    saving_section(ui, &p);
}

fn profile_section(ui: &mut Ui, sampler: &Sampler, p: &PowerInfo, busy: bool) {
    group_box(
        ui,
        RichText::new("Power profile").strong(),
        4.0,
        6.0,
        |ui| {
            let Some(c) = &p.power_profile else {
                ui.weak(
                    "power-profiles-daemon isn't running. Desktops switch between Performance, \
                 Balanced and Power Saver through it; install the power-profiles-daemon package \
                 to get them.",
                );
                return;
            };
            ui.add_enabled_ui(!busy, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for o in &c.options {
                        if ui.selectable_label(*o == c.current, o).clicked() && *o != c.current {
                            sampler.set_power_profile(o.clone());
                        }
                    }
                });
            });
            ui.weak(
                "Set by power-profiles-daemon, which also sets the CPU preference and platform \
             profile below. Doesn't need your password.",
            );
        },
    );
}

fn cpu_section(ui: &mut Ui, sampler: &Sampler, p: &PowerInfo, busy: bool) {
    group_box(ui, RichText::new("CPU").strong(), 4.0, 6.0, |ui| {
        let mut change: Option<(Setting, String)> = None;
        egui::Grid::new("power_cpu")
            .num_columns(2)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                let driver = match &p.pstate_mode {
                    Some(m) => format!("{} ({m} mode)", p.cpufreq_driver),
                    None => p.cpufreq_driver.clone(),
                };
                value_row(ui, "Frequency driver", &driver);
                if let Some(g) = &p.governor {
                    let hint = |n: &str| governor_hint(n, &p.cpufreq_driver);
                    if let Some(v) = choice_row(ui, "Governor", g, !busy, hint) {
                        change = Some((Setting::Governor, v));
                    }
                }
                if let Some(e) = &p.epp
                    && let Some(v) =
                        choice_row(ui, "Energy/performance preference", e, !busy, epp_hint)
                {
                    change = Some((Setting::Epp, v));
                }
                if let Some(on) = p.boost {
                    ui.label("Boost");
                    let mut v = on;
                    let r = ui
                        .add_enabled(
                            !busy,
                            egui::Checkbox::new(&mut v, "Turbo / Precision Boost"),
                        )
                        .on_hover_text("Lets the CPU run above its base clock when it can.");
                    if r.changed() {
                        change = Some((Setting::Boost, if v { "on" } else { "off" }.into()));
                    }
                    ui.end_row();
                }
                if let Some(pp) = &p.platform_profile
                    && let Some(v) = choice_row(
                        ui,
                        "Platform profile",
                        pp,
                        !busy,
                        |_| "The firmware's fan and power limits profile.",
                    )
                {
                    change = Some((Setting::PlatformProfile, v));
                }
            });
        if let Some((setting, value)) = change {
            sampler.set_power_setting(setting, value);
        }
    });
}

fn saving_section(ui: &mut Ui, p: &PowerInfo) {
    group_box(
        ui,
        RichText::new("Power saving and sleep").strong(),
        4.0,
        6.0,
        |ui| {
            egui::Grid::new("power_saving")
                .num_columns(2)
                .spacing([16.0, 6.0])
                .show(ui, |ui| {
                    if let Some(a) = &p.aspm {
                        let text = match aspm_text(&a.current) {
                            "" => a.current.clone(),
                            t => t.to_string(),
                        };
                        value_row(ui, "PCIe link power saving (ASPM)", &text);
                    }
                    if let Some(s) = p.usb_autosuspend_secs {
                        let text = if s < 0 {
                            "off".to_string()
                        } else {
                            format!("after {s} s idle")
                        };
                        value_row(ui, "USB autosuspend", &text);
                    }
                    if let Some(m) = &p.mem_sleep {
                        value_row(ui, "Sleep mode", &sleep_text(&m.current));
                    }
                });
            let wake: Vec<_> = p.wake_devices.iter().filter(|d| d.enabled).collect();
            if !wake.is_empty() {
                ui.add_space(4.0);
                ui.label(format!("Devices that can wake the PC ({}):", wake.len()));
                for d in wake {
                    if d.sysfs.is_empty() {
                        ui.label(format!("• {}", d.name));
                    } else {
                        ui.label(format!("• {} ({})", d.name, d.sysfs));
                    }
                }
            }
        },
    );
}
