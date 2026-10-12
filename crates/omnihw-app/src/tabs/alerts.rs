//! Alerts tab: create and edit alert rules, see which are firing, and review history.

use crate::heat::{ORANGE, RED};
use crate::sampler::{Sampler, Snapshot};
use egui::{RichText, Ui};
use omnihw_core::SystemInfo;
use omnihw_core::alerts::{AlertRule, Condition};
use omnihw_core::sensors::{Tracked, Unit};
use std::time::{SystemTime, UNIX_EPOCH};

/// Status message from the last save attempt.
#[derive(Default)]
pub struct ViewState {
    save_error: Option<String>,
}

fn sensor_name(t: &Tracked) -> String {
    format!("{} / {}", t.reading.group, t.reading.label)
}

fn rule_for(t: &Tracked, condition: Condition, threshold: f64, duration_secs: f64) -> AlertRule {
    AlertRule {
        enabled: true,
        sensor_id: t.reading.id.clone(),
        sensor_name: sensor_name(t),
        unit: t.reading.unit,
        condition,
        threshold,
        duration_secs,
    }
}

/// Common alerts for the sensors this machine has.
fn presets(info: &SystemInfo, snap: &Snapshot) -> Vec<(String, AlertRule)> {
    let mut out = Vec::new();
    let mut add = |label: String, id: &str, cond, threshold, dur| {
        if let Some(t) = snap.table.get(id) {
            out.push((label, rule_for(t, cond, threshold, dur)));
        }
    };
    for g in &info.gpus {
        let pre = g.sensor_prefix();
        add(
            format!("{} over 85 °C", g.name),
            &format!("{pre}/temp"),
            Condition::Above,
            85.0,
            30.0,
        );
    }
    add(
        "CPU usage over 95 % for 2 min".into(),
        "cpu/load/total",
        Condition::Above,
        95.0,
        120.0,
    );
    add(
        "Memory over 90 % for 1 min".into(),
        "mem/load",
        Condition::Above,
        90.0,
        60.0,
    );
    for d in &info.drives {
        add(
            format!("{} over 70 °C", d.name()),
            &format!("{}/temp", d.sensor_prefix()),
            Condition::Above,
            70.0,
            30.0,
        );
    }
    add(
        "Battery below 20 %".into(),
        "battery/charge",
        Condition::Below,
        20.0,
        0.0,
    );
    out
}

fn ago(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    match s {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", s / 60),
        3600..86400 => format!("{} h ago", s / 3600),
        _ => format!("{} days ago", s / 86400),
    }
}

pub fn show(ui: &mut Ui, info: &SystemInfo, sampler: &Sampler, state: &mut ViewState) {
    let snap = sampler.snapshot();
    let mut rules: Vec<AlertRule> = snap.alerts.rules().to_vec();
    let original = rules.clone();
    let active: Vec<bool> = (0..rules.len()).map(|i| snap.alerts.is_active(i)).collect();
    let mut sensors: Vec<&Tracked> = snap.table.entries().iter().collect();
    sensors.sort_by_key(|t| (t.reading.group.clone(), t.reading.label.clone()));

    ui.label(RichText::new("Alerts").size(20.0).strong());
    ui.weak("Get a Windows notification when a sensor stays above or below a limit. Alerts are checked on every sample while Omni-HW is running, on any tab.");
    ui.add_space(6.0);
    let mut test = false;
    ui.horizontal_wrapped(|ui| {
        ui.label("Quick add:");
        for (label, rule) in presets(info, &snap) {
            let exists = rules
                .iter()
                .any(|r| r.sensor_id == rule.sensor_id && r.condition == rule.condition);
            if ui.add_enabled(!exists, egui::Button::new(label)).clicked() {
                rules.push(rule);
            }
        }
    });
    ui.horizontal(|ui| {
        if ui.button("Add custom alert").clicked()
            && let Some(t) = sensors.first()
        {
            rules.push(rule_for(t, Condition::Above, t.reading.value.round(), 10.0));
        }
        test = ui
            .button("Test notification")
            .on_hover_text("Shows a sample Windows notification")
            .clicked();
    });
    ui.add_space(8.0);

    if rules.is_empty() {
        ui.weak("No alerts yet. Use Quick add or Add custom alert.");
    } else {
        let mut remove = None;
        egui::Grid::new("alert_rules")
            .num_columns(7)
            .spacing([10.0, 6.0])
            .striped(true)
            .show(ui, |ui| {
                for h in ["On", "Status", "Sensor", "When", "Limit", "For", ""] {
                    ui.label(RichText::new(h).underline());
                }
                ui.end_row();
                for (i, r) in rules.iter_mut().enumerate() {
                    ui.checkbox(&mut r.enabled, "");
                    if !r.enabled {
                        ui.weak("off");
                    } else if active.get(i).copied().unwrap_or(false) {
                        ui.colored_label(
                            crate::heat::ink(ui.visuals(), RED),
                            RichText::new("FIRING").strong(),
                        );
                    } else {
                        let current = snap
                            .table
                            .get(&r.sensor_id)
                            .map(|t| r.unit.format(t.reading.value));
                        ui.weak(current.map_or("no data".to_string(), |c| format!("ok ({c})")));
                    }
                    egui::ComboBox::from_id_salt(("alert_sensor", i))
                        .width(320.0)
                        .height(420.0)
                        .selected_text(&r.sensor_name)
                        .show_ui(ui, |ui| {
                            for t in &sensors {
                                if ui
                                    .selectable_label(t.reading.id == r.sensor_id, sensor_name(t))
                                    .clicked()
                                {
                                    r.sensor_id = t.reading.id.clone();
                                    r.sensor_name = sensor_name(t);
                                    r.unit = t.reading.unit;
                                }
                            }
                        });
                    egui::ComboBox::from_id_salt(("alert_cond", i))
                        .width(70.0)
                        .selected_text(match r.condition {
                            Condition::Above => "above",
                            Condition::Below => "below",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut r.condition, Condition::Above, "above");
                            ui.selectable_value(&mut r.condition, Condition::Below, "below");
                        });
                    let speed = match r.unit {
                        Unit::Mhz | Unit::Megabytes => 10.0,
                        _ => 0.5,
                    };
                    ui.add(
                        egui::DragValue::new(&mut r.threshold)
                            .speed(speed)
                            .suffix(format!(" {}", r.unit.symbol())),
                    );
                    ui.add(
                        egui::DragValue::new(&mut r.duration_secs)
                            .range(0.0..=3600.0)
                            .speed(1.0)
                            .suffix(" s"),
                    );
                    if ui.small_button("Remove").clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
        if let Some(i) = remove {
            rules.remove(i);
        }
    }
    if let Some(e) = &state.save_error {
        ui.colored_label(crate::heat::ink(ui.visuals(), ORANGE), e);
    }

    ui.add_space(12.0);
    ui.label(RichText::new("History").strong());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    if snap.alert_log.is_empty() {
        ui.weak("No alerts have fired since Omni-HW started.");
    } else {
        egui::Grid::new("alert_log")
            .num_columns(2)
            .spacing([16.0, 4.0])
            .striped(true)
            .show(ui, |ui| {
                for e in snap.alert_log.iter().rev().take(50) {
                    ui.weak(ago(now - e.at_unix));
                    ui.label(e.message());
                    ui.end_row();
                }
            });
    }
    drop(snap);

    if test {
        sampler.test_notification();
    }
    if rules != original {
        state.save_error = sampler.set_alert_rules(rules).err();
    }
}

/// Number of alerts currently firing (for the tab bar indicator).
pub fn firing_count(snap: &Snapshot) -> usize {
    (0..snap.alerts.rules().len())
        .filter(|&i| snap.alerts.is_active(i))
        .count()
}
