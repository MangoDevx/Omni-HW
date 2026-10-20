//! Settings page: appearance, startup, monitoring and file locations.

use super::{group_box, page_title, temp};
use crate::autostart;
use crate::heat::ORANGE;
use crate::pawnio::{self, Installer, Job};
use crate::sampler::Sampler;
use crate::settings::{INTERVALS_MS, NavStyle, PAGES, Settings, Theme, UI_SCALE_RANGE};
use crate::update::{RELEASES_URL, Status, UpdateChecker};
use egui::{RichText, Ui};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// A titled group box.
fn group(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    group_box(ui, RichText::new(title).size(16.0).strong(), 4.0, 8.0, add);
}

fn temp_id(key: &str) -> egui::Id {
    egui::Id::new(("settings", key))
}

/// Transient per-page text kept in egui memory (editing buffer, last error).
fn temp_string(ui: &Ui, key: &str) -> String {
    temp::get(ui.ctx(), temp_id(key)).unwrap_or_default()
}

fn set_temp_string(ui: &Ui, key: &str, v: String) {
    temp::set(ui.ctx(), temp_id(key), v);
}

/// Where reports go ([`crate::export::report_dir`]). Resolving it checks the file system, so the
/// answer is kept for a couple of seconds (or until the chosen folder changes) instead of being
/// re-checked on every frame.
fn report_dir(ui: &Ui, chosen: &Option<PathBuf>) -> PathBuf {
    const FRESH: Duration = Duration::from_secs(2);
    type Cached = (Option<PathBuf>, PathBuf, Instant);
    let id = temp_id("report_dir");
    if let Some((for_dir, dir, at)) = temp::get::<Cached>(ui.ctx(), id)
        && for_dir == *chosen
        && at.elapsed() < FRESH
    {
        return dir;
    }
    let dir = crate::export::report_dir();
    temp::set::<Cached>(ui.ctx(), id, (chosen.clone(), dir.clone(), Instant::now()));
    dir
}

/// Parse a typed UI-scale percentage ("120", "120%", "120 %") into a scale factor.
fn parse_percent(text: &str) -> Option<f64> {
    let v: f64 = text.trim().trim_end_matches('%').trim().parse().ok()?;
    Some(v / 100.0)
}

/// Returns true when a setting changed; the caller saves and applies them.
pub fn show(
    ui: &mut Ui,
    s: &mut Settings,
    sampler: &Sampler,
    updates: &UpdateChecker,
    pawnio: &Installer,
) -> bool {
    let before = s.clone();
    page_title(ui, "Settings");
    ui.weak("Changes apply immediately and are saved automatically.");
    ui.add_space(8.0);

    group(ui, "Appearance", |ui| {
        egui::Grid::new("settings_appearance")
            .num_columns(2)
            .spacing([16.0, 8.0])
            .show(ui, |ui| {
                ui.label("Theme");
                egui::ComboBox::from_id_salt("theme")
                    .selected_text(match s.theme {
                        Theme::System => "Match Windows",
                        Theme::Dark => "Dark",
                        Theme::Light => "Light",
                        Theme::Retro => "Retro (2000s)",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut s.theme, Theme::System, "Match Windows");
                        ui.selectable_value(&mut s.theme, Theme::Dark, "Dark");
                        ui.selectable_value(&mut s.theme, Theme::Light, "Light");
                        ui.selectable_value(&mut s.theme, Theme::Retro, "Retro (2000s)");
                    });
                ui.end_row();

                ui.label("UI scale");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Slider::new(&mut s.ui_scale, UI_SCALE_RANGE)
                            .step_by(0.05)
                            .custom_formatter(|v, _| format!("{:.0} %", v * 100.0))
                            // Typed values are percentages too ("120", "120%", "120 %").
                            .custom_parser(parse_percent),
                    );
                    if ui.small_button("Reset").clicked() {
                        s.ui_scale = 1.0;
                    }
                });
                ui.end_row();

                ui.label("Navigation bar");
                ui.vertical(|ui| {
                    ui.radio_value(&mut s.nav_style, NavStyle::Grouped, "Sidebar");
                    ui.weak("Every page listed down the left side, sorted into categories.");
                    ui.radio_value(&mut s.nav_style, NavStyle::Classic, "Classic");
                    ui.weak(
                        "Every page as its own tab (may wrap onto two lines in small windows).",
                    );
                });
                ui.end_row();
            });
    });

    group(ui, "Startup", |ui| {
        egui::Grid::new("settings_startup")
            .num_columns(2)
            .spacing([16.0, 8.0])
            .show(ui, |ui| {
                ui.label("Start page");
                egui::ComboBox::from_id_salt("start_page")
                    .selected_text(s.start_page_name())
                    .show_ui(ui, |ui| {
                        for (key, name) in PAGES {
                            if ui.selectable_label(s.start_page == key, name).clicked() {
                                s.start_page = key.to_string();
                            }
                        }
                    });
                ui.end_row();

                ui.label("Windows startup");
                ui.vertical(|ui| {
                    let mut launch = s.launch_at_startup;
                    let mut minimized = s.start_minimized;
                    let a = ui.checkbox(&mut launch, "Launch Omni-HW when I sign in to Windows");
                    let b = ui
                        .add_enabled(launch, egui::Checkbox::new(&mut minimized, "Start minimized"));
                    if a.changed() || b.changed() {
                        match autostart::set(launch, minimized) {
                            Ok(()) => {
                                s.launch_at_startup = launch;
                                s.start_minimized = minimized;
                                set_temp_string(ui, "autostart_err", String::new());
                            }
                            Err(e) => set_temp_string(ui, "autostart_err", e),
                        }
                    }
                    let err = temp_string(ui, "autostart_err");
                    if !err.is_empty() {
                        ui.colored_label(crate::heat::ink(ui.visuals(), ORANGE), err);
                    }
                    ui.weak("Launches at sign-in never show the administrator prompt.");
                });
                ui.end_row();

                ui.label("System tray");
                ui.vertical(|ui| {
                    ui.checkbox(&mut s.minimize_to_tray, "Minimize to the system tray");
                    ui.weak("Minimizing hides Omni-HW next to the clock; click its icon to bring it back. Monitoring and alerts keep running.");
                });
                ui.end_row();

                ui.label("Administrator");
                ui.vertical(|ui| {
                    ui.checkbox(
                        &mut s.ask_admin_at_start,
                        "Ask for administrator rights at start when SATA drives are present",
                    );
                    ui.weak(
                        "Reading SATA drive health (SMART) needs administrator rights. NVMe drives don't.",
                    );
                });
                ui.end_row();
            });
    });

    group(ui, "Monitoring", |ui| {
        egui::Grid::new("settings_monitoring")
            .num_columns(2)
            .spacing([16.0, 8.0])
            .show(ui, |ui| {
                ui.label("Sampling interval");
                ui.vertical(|ui| {
                    egui::ComboBox::from_id_salt("interval")
                        .selected_text(format!("{} ms", s.sample_interval_ms))
                        .show_ui(ui, |ui| {
                            for ms in INTERVALS_MS {
                                ui.selectable_value(
                                    &mut s.sample_interval_ms,
                                    ms,
                                    format!("{ms} ms"),
                                );
                            }
                        });
                    ui.weak("How often sensors are read. Shorter is more detailed but uses a little more CPU.");
                });
                ui.end_row();

                ui.label("Alert notifications");
                ui.vertical(|ui| {
                    ui.checkbox(
                        &mut s.alert_notifications,
                        "Show Windows notifications for alerts",
                    );
                    ui.weak("Fired alerts always appear in the app; this controls the Windows notification and taskbar flash.");
                });
                ui.end_row();
            });
    });

    group(ui, "Files", |ui| {
        let current = report_dir(ui, &s.data_dir);
        ui.horizontal_wrapped(|ui| {
            ui.label("Recordings and reports are saved in:");
            ui.label(RichText::new(current.display().to_string()).strong());
        });
        let mut edit = temp_string(ui, "data_dir_edit");
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut edit)
                    .hint_text("Paste a folder path")
                    .desired_width(360.0),
            );
            let picker: crate::folder_picker::FolderPicker =
                temp::get(ui.ctx(), egui::Id::new("data_dir_picker")).unwrap_or_default();
            temp::set(ui.ctx(), egui::Id::new("data_dir_picker"), picker.clone());
            if let Some(p) = picker.take() {
                s.data_dir = Some(p);
                edit.clear();
                set_temp_string(ui, "data_dir_err", String::new());
            }
            if ui
                .add_enabled(!picker.is_open(), egui::Button::new("Browse…"))
                .on_hover_text("Choose a folder")
                .clicked()
            {
                picker.open(ui.ctx(), current.clone());
            }
            if ui.button("Apply").clicked() {
                let p = PathBuf::from(edit.trim());
                if edit.trim().is_empty() {
                    set_temp_string(ui, "data_dir_err", "Enter a folder path first.".into());
                } else if !p.is_dir() {
                    set_temp_string(
                        ui,
                        "data_dir_err",
                        format!("Folder not found: {}", p.display()),
                    );
                } else {
                    s.data_dir = Some(p);
                    edit.clear();
                    set_temp_string(ui, "data_dir_err", String::new());
                }
            }
        });
        set_temp_string(ui, "data_dir_edit", edit);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    s.data_dir.is_some(),
                    egui::Button::new("Reset to Documents"),
                )
                .clicked()
            {
                s.data_dir = None;
            }
            if ui.button("Open folder").clicked() {
                crate::export::open_in_explorer(&current, false);
            }
        });
        let err = temp_string(ui, "data_dir_err");
        if !err.is_empty() {
            ui.colored_label(crate::heat::ink(ui.visuals(), ORANGE), err);
        }
    });

    group(ui, "Health Check", |ui| {
        ui.checkbox(&mut s.show_gaming, "Show gaming suggestions");
        ui.weak("When off, the Gaming category and its suggestions are hidden in Health Check and on Overview.");
        ui.add_space(4.0);
        ui.checkbox(
            &mut s.remember_dismissed,
            "Keep dismissed issues dismissed after restarting Omni-HW",
        );
        ui.weak("When off, dismissed issues come back the next time Omni-HW starts.");
        let n = crate::dismissed::count();
        if ui
            .add_enabled(
                n > 0,
                egui::Button::new(format!("Restore dismissed issues ({n})")),
            )
            .clicked()
        {
            crate::dismissed::restore_all();
        }
    });

    group(ui, "Extra sensors (PawnIO)", |ui| {
        pawnio_group(ui, s, sampler, pawnio)
    });

    group(ui, "Updates", |ui| {
        ui.horizontal_wrapped(|ui| {
            let status = updates.status();
            let checking = status == Status::Checking;
            if ui
                .add_enabled(!checking, egui::Button::new("Check for updates"))
                .clicked()
            {
                updates.start(ui.ctx());
            }
            // Opens the latest release notes in the browser; the app itself sends nothing.
            if ui
                .button("What's new")
                .on_hover_text("Open the latest release notes on GitHub in your browser")
                .clicked()
            {
                ui.ctx().open_url(egui::OpenUrl::new_tab(RELEASES_URL));
            }
            let version = env!("CARGO_PKG_VERSION");
            match status {
                Status::Idle => {
                    ui.weak(format!("You have version {version}."));
                }
                Status::Checking => {
                    ui.spinner();
                    ui.label("Checking GitHub...");
                }
                Status::UpToDate => {
                    ui.label(format!("Omni-HW {version} is the latest version."));
                }
                Status::Available(latest) => {
                    ui.label(
                        RichText::new(format!(
                            "Version {latest} is available (you have {version})."
                        ))
                        .strong(),
                    );
                    ui.hyperlink_to("Open the download page", RELEASES_URL);
                }
                Status::AheadOfRelease(latest) => {
                    ui.label(format!(
                        "You have {version}, newer than the latest release ({latest})."
                    ));
                }
                Status::Failed(e) => {
                    ui.colored_label(crate::heat::ink(ui.visuals(), ORANGE), e);
                }
            }
        });
        ui.checkbox(
            &mut s.check_updates_at_start,
            "Check for updates when Omni-HW starts",
        );
        ui.weak(
            "Update checks are Omni-HW's only network request and happen only when you click the button or turn this on. \
             They ask GitHub for the latest release's version number; nothing about your PC is sent.",
        );
    });

    if let Some(p) = Settings::path() {
        ui.weak(format!("Settings are stored in {}", p.display()));
    }

    let changed = *s != before;
    if changed {
        sampler.set_interval_ms(s.sample_interval_ms);
        sampler.set_alert_notifications(s.alert_notifications);
        sampler.set_use_pawnio(s.use_pawnio);
        crate::export::set_data_dir(s.data_dir.clone());
    }
    changed
}

/// The optional extra sensors (PawnIO driver + Omni-HW sensor service): state, setup and removal
/// buttons, the on/off setting, and the driver's licences.
fn pawnio_group(ui: &mut Ui, s: &mut Settings, sampler: &Sampler, installer: &Installer) {
    use omnihw_core::sensor_service::ServiceState;
    ui.label(
        "Per-core CPU temperatures, CPU voltage, and motherboard fan speeds, temperatures and voltages, read through PawnIO, a small signed open-source driver used by several hardware monitors. It's optional: everything else works without it.",
    );
    ui.weak(
        "Setup installs PawnIO and a small Omni-HW sensor service, which is the only part that talks to the driver. It only reads sensors (it never changes clocks, voltages or fan speeds) and only hands readings out, so Omni-HW itself doesn't need administrator rights.",
    );
    ui.add_space(4.0);

    if let Some(set_up) = installer.take_finished() {
        // Turn the readings on after a successful setup, and re-check the service either way.
        if set_up {
            s.use_pawnio = true;
        }
        sampler.refresh_sensor_service();
    }
    let job = installer.job();
    let status = sampler.snapshot().sensor_service.clone();
    let warn = crate::heat::ink(ui.visuals(), ORANGE);
    let pawnio = status
        .as_ref()
        .and_then(|st| st.pawnio_version.clone())
        .map(|v| format!("PawnIO {v}"));
    // (offer setup / repair, offer removal)
    let (setup_label, can_remove) = match status.as_ref().map(|st| &st.state) {
        None => {
            ui.weak("Checking...");
            (None, false)
        }
        Some(ServiceState::NotSetUp) => {
            match &pawnio {
                Some(p) => ui.label(format!(
                    "{p} is installed (by another app), but Omni-HW's sensor service isn't set up."
                )),
                None => ui.label("Not set up."),
            };
            (Some("Set up extra sensors..."), false)
        }
        Some(ServiceState::NotRunning(e)) => {
            ui.colored_label(warn, format!("The sensor service isn't answering: {e}."));
            (Some("Repair..."), true)
        }
        Some(ServiceState::Outdated { version }) => {
            ui.colored_label(
                warn,
                format!("The sensor service is from Omni-HW {version} and needs updating."),
            );
            (Some("Update sensor service..."), true)
        }
        Some(ServiceState::Ready {
            sources, errors, ..
        }) => {
            let via = pawnio.unwrap_or_else(|| "PawnIO".into());
            if !s.use_pawnio {
                ui.label(format!("Set up ({via}), but turned off below."));
            } else if sources.is_empty() {
                ui.colored_label(
                    warn,
                    format!(
                        "The sensor service found no sensors: {}.",
                        errors.join("; ")
                    ),
                );
            } else {
                ui.label(format!("Working ({via}): {}.", sources.join(", ")));
            }
            (None, true)
        }
    };

    ui.horizontal_wrapped(|ui| {
        let busy = matches!(job, Job::Running(_));
        if let Some(label) = setup_label
            && ui
                .add_enabled(!busy, egui::Button::new(label))
                .on_hover_text(
                    "Installs PawnIO (if needed) and the Omni-HW sensor service. Windows asks for administrator permission once.",
                )
                .clicked()
        {
            installer.start(ui.ctx(), true);
        }
        if can_remove
            && ui
                .add_enabled(!busy, egui::Button::new("Remove extra sensors..."))
                .on_hover_text(
                    "Removes the sensor service, and PawnIO too if Omni-HW installed it. Asks for administrator permission.",
                )
                .clicked()
        {
            installer.start(ui.ctx(), false);
        }
        match &job {
            Job::Idle => {}
            Job::Running(what) => {
                ui.spinner();
                ui.label(format!("{what}..."));
            }
            Job::Done(Ok(msg)) => {
                ui.label(msg);
            }
            Job::Done(Err(e)) => {
                ui.colored_label(warn, e);
            }
        }
    });
    ui.checkbox(&mut s.use_pawnio, "Use extra sensors");

    ui.collapsing("Licences and source code", |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.hyperlink_to("pawnio.eu", pawnio::SITE_URL);
            ui.hyperlink_to("Driver source", pawnio::DRIVER_SOURCE_URL);
            ui.hyperlink_to("Installer source", pawnio::SETUP_SOURCE_URL);
            ui.hyperlink_to("Modules source", pawnio::MODULES_SOURCE_URL);
        });
        ui.weak(
            "Omni-HW includes the official, unmodified PawnIO installer (GNU GPL 2 with an exception for programs that use its device interface, as Omni-HW does) and PawnIO modules (GNU LGPL 2.1).",
        );
        for (title, text) in [
            ("PawnIO driver licence (GPL 2)", pawnio::DRIVER_LICENSE),
            ("PawnIO modules licence (LGPL 2.1)", pawnio::MODULES_LICENSE),
        ] {
            ui.collapsing(title, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(title)
                    .max_height(260.0)
                    .show(ui, |ui| ui.monospace(text));
            });
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn percent_parsing() {
        assert_eq!(super::parse_percent("120"), Some(1.2));
        assert_eq!(super::parse_percent(" 85 % "), Some(0.85));
        assert_eq!(super::parse_percent("150%"), Some(1.5));
        assert_eq!(super::parse_percent("abc"), None);
    }
}
