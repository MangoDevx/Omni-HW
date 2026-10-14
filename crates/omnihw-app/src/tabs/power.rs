//! Power page: power plans, timers, buttons, processor limits, device power saving, wake
//! sources and power requests, with in-place editing. Changes are requested here and applied
//! by `dispatch` (outside the snapshot lock) on a worker thread; the page itself only reads the
//! snapshot.

use crate::heat::{GREEN, ORANGE};
use crate::sampler::{Sampler, Snapshot};
use egui::{Id, RichText, Ui};
use omnihw_core::power::{
    Action, AdapterKind, Kind, PERCENT_PRESETS, PowerInfo, Setting, Source, TIMEOUT_PRESETS,
};
use std::sync::OnceLock;

const PENDING: &str = "power_pending";
const CONFIRM: &str = "power_confirm";
const REFRESH: &str = "power_refresh";
const QUERY: &str = "power_query";

fn elevated() -> bool {
    static E: OnceLock<bool> = OnceLock::new();
    *E.get_or_init(omnihw_core::win::is_elevated)
}

/// Ask for `action` to be applied (after a confirmation if it's disruptive).
pub fn request(ctx: &egui::Context, action: Action) {
    ctx.data_mut(|d| d.insert_temp(Id::new(PENDING), Some(action)));
}

fn flag(ctx: &egui::Context, key: &str) {
    ctx.data_mut(|d| d.insert_temp(Id::new(key), true));
}

fn take_flag(ctx: &egui::Context, key: &str) -> bool {
    ctx.data_mut(|d| d.remove_temp::<bool>(Id::new(key)))
        .unwrap_or(false)
}

fn take_action(ctx: &egui::Context, key: &str) -> Option<Action> {
    ctx.data_mut(|d| d.remove_temp::<Option<Action>>(Id::new(key)))
        .flatten()
}

fn admin_hint(a: &Action) -> &'static str {
    if a.needs_admin() && !elevated() {
        "Asks for administrator permission"
    } else {
        "Applies the change now"
    }
}

/// A "Fix" button for a Health Check finding.
pub fn fix_button(ui: &mut Ui, action: &Action) {
    if ui
        .button(RichText::new(format!("Fix: {}", action.label())).strong())
        .on_hover_text(admin_hint(action))
        .clicked()
    {
        request(ui.ctx(), action.clone());
    }
}

/// Run requested changes and show the confirmation dialog. Call once per frame, outside the
/// snapshot lock. Returns a status message when a change finished.
pub fn dispatch(ctx: &egui::Context, sampler: &Sampler) -> Option<String> {
    if take_flag(ctx, REFRESH) {
        sampler.refresh_power();
    }
    if take_flag(ctx, QUERY) {
        sampler.power_query_admin();
    }
    if let Some(a) = take_action(ctx, PENDING) {
        if a.confirm_text().is_some() {
            ctx.data_mut(|d| d.insert_temp(Id::new(CONFIRM), Some(a)));
        } else {
            sampler.power_action(a);
        }
    }
    if let Some(a) = ctx.data(|d| d.get_temp::<Option<Action>>(Id::new(CONFIRM)).flatten()) {
        let mut close = None;
        egui::Modal::new(Id::new("power_confirm_modal")).show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.label(RichText::new(a.label()).size(16.0).strong());
            ui.add_space(6.0);
            ui.add(egui::Label::new(a.confirm_text().unwrap_or_default()).wrap());
            if a.needs_admin() && !elevated() {
                ui.add_space(4.0);
                ui.weak("Windows will ask for administrator permission.");
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Apply").strong()).clicked() {
                    close = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    close = Some(false);
                }
            });
        });
        if let Some(apply) = close {
            take_action(ctx, CONFIRM);
            if apply {
                sampler.power_action(a);
            }
        }
    }
    sampler.snapshot().power_msg.take()
}

pub fn show(ui: &mut Ui, snap: &Snapshot) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Power").size(20.0).strong());
        if snap.power_busy {
            ui.spinner();
            ui.weak("Working...");
        } else if ui
            .small_button("Refresh")
            .on_hover_text("Read power settings, devices and wake history again")
            .clicked()
        {
            flag(ui.ctx(), REFRESH);
        }
    });
    let Some(p) = &snap.power else {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Reading power settings...");
        });
        return;
    };
    ui.weak(
        "Changes apply to the active power plan right away. Some need administrator permission; Windows asks when needed.",
    );
    ui.add_space(8.0);
    ui.add_enabled_ui(!snap.power_busy, |ui| {
        plans(ui, p);
        settings_group(
            ui,
            p,
            "Sleep and display",
            &[
                Setting::DisplayOff,
                Setting::Sleep,
                Setting::Hibernate,
                Setting::DiskOff,
            ]
            .into_iter()
            .filter(|s| *s != Setting::Hibernate || p.caps.hibernate)
            .collect::<Vec<_>>(),
        );
        let mut buttons = vec![Setting::PowerButton];
        if p.caps.sleep_button {
            buttons.push(Setting::SleepButton);
        }
        if p.caps.lid {
            buttons.push(Setting::LidAction);
        }
        settings_group(ui, p, "Power buttons and lid", &buttons);
        settings_group(
            ui,
            p,
            "Processor",
            &[Setting::ProcessorMin, Setting::ProcessorMax],
        );
        let mut adv = vec![
            Setting::UsbSelectiveSuspend,
            Setting::PcieAspm,
            Setting::WakeTimers,
        ];
        if p.adapters.iter().any(|a| a.kind == AdapterKind::WiFi) {
            adv.push(Setting::WirelessPowerSaving);
        }
        if p.is_portable() {
            adv.push(Setting::BatterySaverThreshold);
        }
        settings_group(ui, p, "Advanced", &adv);
        if !p.sensitive_usb.is_empty() {
            let names: Vec<String> = p
                .sensitive_usb
                .iter()
                .map(|(n, k)| format!("{n} ({})", k.name()))
                .collect();
            ui.weak(format!(
                "USB devices that may drop out with selective suspend: {}",
                names.join(", ")
            ));
            ui.add_space(6.0);
        }
        startup(ui, p);
        adapters(ui, p);
        waking(ui, p);
        requests(ui, p);
    });
}

fn group(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    ui.group(|ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).strong());
        ui.add_space(2.0);
        body(ui);
    });
    ui.add_space(6.0);
}

fn plans(ui: &mut Ui, p: &PowerInfo) {
    group(ui, "Power plan", |ui| {
        if p.schemes.is_empty() {
            ui.weak("No power plans found.");
            return;
        }
        ui.horizontal_wrapped(|ui| {
            for s in &p.schemes {
                let active = p.active.as_deref() == Some(s.guid.as_str());
                let name = if s.name.is_empty() {
                    "Unnamed plan"
                } else {
                    &s.name
                };
                if ui
                    .selectable_label(active, name)
                    .on_hover_text(if active {
                        "Active plan"
                    } else {
                        "Switch to this plan"
                    })
                    .clicked()
                    && !active
                {
                    request(ui.ctx(), Action::SetScheme(s.guid.clone()));
                }
            }
        });
    });
}

/// Rows of AC (and battery, on laptops) value pickers.
fn settings_group(ui: &mut Ui, p: &PowerInfo, title: &str, settings: &[Setting]) {
    let portable = p.is_portable();
    group(ui, title, |ui| {
        egui::Grid::new(("power_grid", title))
            .num_columns(if portable { 3 } else { 2 })
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                if portable {
                    ui.label("");
                    ui.label(RichText::new("Plugged in").underline());
                    ui.label(RichText::new("On battery").underline());
                    ui.end_row();
                }
                for &s in settings {
                    let v = p.get(s);
                    if v.ac.is_none() && v.dc.is_none() {
                        continue;
                    }
                    ui.label(s.label());
                    // Battery saver only matters on battery.
                    if s != Setting::BatterySaverThreshold {
                        value_combo(ui, s, Source::Ac, v.ac);
                    } else {
                        ui.weak("-");
                    }
                    if portable {
                        value_combo(ui, s, Source::Dc, v.dc);
                    }
                    ui.end_row();
                }
            });
    });
}

/// Choices offered for a setting, including the current value if it isn't a preset.
fn options(s: Setting, current: u32) -> Vec<u32> {
    let mut v: Vec<u32> = match s.kind() {
        Kind::Seconds => TIMEOUT_PRESETS.to_vec(),
        Kind::Percent => PERCENT_PRESETS.to_vec(),
        Kind::Choice(c) => c.iter().map(|(v, _)| *v).collect(),
    };
    if !v.contains(&current) {
        v.push(current);
    }
    if s.kind() == Kind::Seconds {
        // Ascending, with "Never" (0) last.
        v.sort_by_key(|&x| if x == 0 { u32::MAX } else { x });
    } else {
        v.sort_unstable();
    }
    v
}

fn value_combo(ui: &mut Ui, s: Setting, source: Source, current: Option<u32>) {
    let Some(cur) = current else {
        ui.weak("-");
        return;
    };
    egui::ComboBox::from_id_salt(("power_value", s.key(), source == Source::Ac))
        .selected_text(s.format(cur))
        .show_ui(ui, |ui| {
            for v in options(s, cur) {
                if ui.selectable_label(v == cur, s.format(v)).clicked() && v != cur {
                    request(
                        ui.ctx(),
                        Action::SetValue {
                            setting: s,
                            source,
                            value: v,
                        },
                    );
                }
            }
        });
}

fn on_off(ui: &mut Ui, on: bool, on_text: &str, off_text: &str) {
    if on {
        ui.label(RichText::new(on_text).color(crate::heat::ink(ui.visuals(), ORANGE)));
    } else {
        ui.label(RichText::new(off_text).color(crate::heat::ink(ui.visuals(), GREEN)));
    }
}

fn action_button(ui: &mut Ui, text: &str, action: Action) {
    if ui
        .small_button(text)
        .on_hover_text(admin_hint(&action))
        .clicked()
    {
        request(ui.ctx(), action);
    }
}

fn startup(ui: &mut Ui, p: &PowerInfo) {
    group(ui, "Startup and standby", |ui| {
        egui::Grid::new("power_startup")
            .num_columns(3)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                ui.label("Fast Startup");
                match p.fast_startup {
                    Some(on) => {
                        let effective = p.fast_startup_effective() == Some(true);
                        ui.label(match (on, effective) {
                            (true, true) => "On",
                            (true, false) => "On (unused: hibernation is off)",
                            _ => "Off",
                        });
                        action_button(
                            ui,
                            if on { "Turn off" } else { "Turn on" },
                            Action::FastStartup(!on),
                        );
                    }
                    None => {
                        ui.weak("-");
                        ui.label("");
                    }
                }
                ui.end_row();
                ui.label("Hibernation");
                ui.label(match (p.caps.hibernate, p.caps.hiberfile) {
                    (true, true) => "On",
                    (true, false) => "Off",
                    _ => "Not supported",
                });
                ui.label("");
                ui.end_row();
                ui.label("Standby");
                ui.label(if p.caps.modern_standby {
                    "Modern Standby (S0 low power idle)"
                } else if p.caps.s3 {
                    "Sleep (S3)"
                } else {
                    "-"
                });
                ui.label("");
                ui.end_row();
            });
    });
}

fn adapters(ui: &mut Ui, p: &PowerInfo) {
    if p.adapters.is_empty() {
        return;
    }
    group(ui, "Network and Bluetooth adapters", |ui| {
        egui::Grid::new("power_adapters")
            .num_columns(3)
            .spacing([16.0, 4.0])
            .striped(true)
            .show(ui, |ui| {
                ui.label(RichText::new("Adapter").underline());
                ui.label(RichText::new("Windows may turn it off").underline());
                ui.label(RichText::new("Energy-Efficient Ethernet").underline());
                ui.end_row();
                for a in &p.adapters {
                    ui.label(format!("{} ({})", a.name, a.kind.name()));
                    ui.horizontal(|ui| match a.allow_turn_off {
                        Some(on) => {
                            on_off(ui, on, "Yes", "No");
                            action_button(
                                ui,
                                if on { "Stop" } else { "Allow" },
                                Action::AllowTurnOff {
                                    instance_id: a.instance_id.clone(),
                                    allow: !on,
                                },
                            );
                        }
                        None => {
                            ui.weak("-");
                        }
                    });
                    ui.horizontal(|ui| match (&a.eee, &a.class_key) {
                        (Some(e), Some(k)) => {
                            on_off(ui, e.enabled, "On", "Off");
                            action_button(
                                ui,
                                if e.enabled { "Turn off" } else { "Turn on" },
                                Action::NicProperty {
                                    class_key: k.clone(),
                                    value: e.value.clone(),
                                    data: if e.enabled { "0" } else { "1" }.into(),
                                },
                            );
                        }
                        _ => {
                            ui.weak("-");
                        }
                    });
                    ui.end_row();
                }
            });
    });
}

fn waking(ui: &mut Ui, p: &PowerInfo) {
    group(ui, "Waking", |ui| {
        egui::Grid::new("power_wake")
            .num_columns(2)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                ui.label("Last wake from sleep");
                ui.label(match p.last_wake() {
                    Some(w) => format!("{} ({} UTC)", w.source.describe(), w.time),
                    None => "None in the last 2 weeks".into(),
                });
                ui.end_row();
                ui.label("Allow wake timers");
                let v = p.get(Setting::WakeTimers);
                ui.label(match (v.ac, v.dc, p.is_portable()) {
                    (Some(a), Some(d), true) => format!(
                        "{} plugged in, {} on battery",
                        Setting::WakeTimers.format(a),
                        Setting::WakeTimers.format(d)
                    ),
                    (Some(a), _, _) => Setting::WakeTimers.format(a),
                    _ => "-".into(),
                });
                ui.end_row();
            });
        ui.add_space(4.0);
        ui.label(RichText::new("Devices that can wake the PC").strong());
        if p.wake_armed.is_empty() {
            ui.weak("None");
        }
        for d in &p.wake_armed {
            ui.horizontal(|ui| {
                ui.label(d);
                action_button(ui, "Disarm", Action::DisarmWake(d.clone()));
            });
        }
        ui.add_space(4.0);
        ui.label(RichText::new("Active wake timers").strong());
        match &p.wake_timers {
            Some(t) if t.is_empty() => {
                ui.weak("None");
            }
            Some(t) => {
                for x in t {
                    ui.add(egui::Label::new(x).wrap());
                }
            }
            None => check_now(ui),
        }
        let history: Vec<_> = p.wakes.iter().collect();
        if !history.is_empty() {
            egui::CollapsingHeader::new(format!("Wake history, last 2 weeks ({})", history.len()))
                .id_salt("power_wake_history")
                .show(ui, |ui| {
                    egui::Grid::new("power_wake_hist")
                        .num_columns(2)
                        .spacing([16.0, 2.0])
                        .striped(true)
                        .show(ui, |ui| {
                            for w in history {
                                ui.label(format!("{} UTC", w.time));
                                ui.label(if w.fast_startup {
                                    "Start-up from Fast Startup shutdown".to_string()
                                } else {
                                    w.source.describe()
                                });
                                ui.end_row();
                            }
                        });
                });
        }
    });
}

fn check_now(ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.weak("Needs administrator rights to read.");
        if ui
            .small_button("Check now")
            .on_hover_text(if elevated() {
                "Read now"
            } else {
                "Asks for administrator permission"
            })
            .clicked()
        {
            flag(ui.ctx(), QUERY);
        }
    });
}

fn requests(ui: &mut Ui, p: &PowerInfo) {
    group(ui, "What's keeping the PC awake", |ui| match &p.requests {
        None => check_now(ui),
        Some(r) if r.is_empty() => {
            ui.label("Nothing: no app or driver is preventing sleep.");
            if ui.small_button("Check again").clicked() {
                flag(ui.ctx(), QUERY);
            }
        }
        Some(r) => {
            egui::Grid::new("power_requests")
                .num_columns(3)
                .spacing([16.0, 2.0])
                .striped(true)
                .show(ui, |ui| {
                    ui.label(RichText::new("Keeps awake").underline());
                    ui.label(RichText::new("Requested by").underline());
                    ui.label(RichText::new("Reason").underline());
                    ui.end_row();
                    for x in r {
                        ui.label(match x.kind.as_str() {
                            "DISPLAY" => "Display",
                            "SYSTEM" => "System",
                            "AWAYMODE" => "Away mode",
                            "EXECUTION" => "Execution",
                            k => k,
                        });
                        ui.label(format!("{} ({})", x.short_name(), x.origin.to_lowercase()))
                            .on_hover_text(&x.name);
                        ui.weak(&x.reason);
                        ui.end_row();
                    }
                });
            if ui.small_button("Check again").clicked() {
                flag(ui.ctx(), QUERY);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::options;
    use omnihw_core::power::Setting;

    #[test]
    fn options_keep_custom_values() {
        let o = options(Setting::Sleep, 420);
        assert!(o.contains(&420));
        assert_eq!(o.last(), Some(&0), "Never is last");
        assert!(o.windows(2).take(o.len() - 2).all(|w| w[0] < w[1]));
        assert_eq!(options(Setting::PcieAspm, 1), vec![0, 1, 2]);
        assert!(options(Setting::ProcessorMax, 42).contains(&42));
    }
}
