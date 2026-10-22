//! Windows settings checks: pending restart, Memory Integrity, GPU scheduling, Game Mode and
//! the pagefile.

use super::helpers::{finding, guided};
use super::{Category, Context, Level, Outcome, Step};

pub(super) fn pending_restart(ctx: &Context) -> Outcome {
    let reasons = &ctx.info.windows.pending_reboot;
    if reasons.is_empty() {
        return Outcome::Pass("No restart pending".into());
    }
    Outcome::Findings(vec![guided(
        Level::Info,
        Category::System,
        "Windows is waiting for a restart",
        format!(
            "A restart is pending for: {}. Updates aren't fully applied until then.",
            reasons.join(", ")
        ),
        vec![Step::settings(
            "Restart (not Shut down) when convenient, or schedule it in Settings > Windows Update.",
            "ms-settings:windowsupdate",
        )],
    )])
}

pub(super) fn memory_integrity(ctx: &Context) -> Outcome {
    let w = &ctx.info.windows;
    match w.hvci_running {
        None => Outcome::Skipped("Memory Integrity status not available".into()),
        Some(false) => Outcome::Pass(
            if w.vbs_configured == Some(true) {
                "Off (VBS configured)"
            } else {
                "Off"
            }
            .into(),
        ),
        Some(true) => Outcome::Findings(vec![guided(
            Level::Info,
            Category::System,
            "Memory Integrity (VBS) is on",
            "Core isolation's Memory Integrity protects the kernel against malware, but costs some performance in CPU-heavy games (commonly a few percent, up to about 10 %).",
            vec![
                Step::new(
                    "Keep it on unless you need the last few percent of gaming performance; it is an important security feature.",
                ),
                Step::settings(
                    "To change it: Windows Security > Device security > Core isolation > Memory integrity (restart required).",
                    "windowsdefender://coreisolation",
                ),
            ],
        )
        .security()]),
    }
}

pub(super) fn gpu_scheduling(ctx: &Context) -> Outcome {
    match ctx.info.windows.hags {
        None => Outcome::Skipped("Not configured; Windows uses the driver's default".into()),
        Some(true) => Outcome::Pass("On".into()),
        Some(false) => Outcome::Findings(vec![guided(
            Level::Info,
            Category::System,
            "Hardware-accelerated GPU scheduling is off",
            "Hardware GPU scheduling can lower latency slightly and is required for DLSS Frame Generation on NVIDIA GPUs.",
            vec![Step::settings(
                "Turn it on in Settings > System > Display > Graphics > Change default graphics settings (restart required).",
                "ms-settings:display-advancedgraphics",
            )],
        )]),
    }
}

pub(super) fn game_mode(ctx: &Context) -> Outcome {
    match ctx.info.windows.game_mode {
        None => Outcome::Skipped("Game Mode setting not available".into()),
        Some(true) => Outcome::Pass("On".into()),
        Some(false) => Outcome::Findings(vec![guided(
            Level::Info,
            Category::System,
            "Game Mode is off",
            "Game Mode stops Windows Update from installing drivers or showing restart prompts during games, and prioritizes the game.",
            vec![Step::settings(
                "Turn it on in Settings > Gaming > Game Mode.",
                "ms-settings:gaming-gamemode",
            )],
        )]),
    }
}

pub(super) fn pagefile(ctx: &Context) -> Outcome {
    match ctx.info.windows.pagefile_disabled {
        None => Outcome::Skipped("Pagefile setting not available".into()),
        Some(false) => Outcome::Pass("Enabled".into()),
        Some(true) => Outcome::Findings(vec![finding(
            Level::Warning,
            Category::System,
            "The pagefile is disabled",
            "Without a pagefile, apps and games crash with out-of-memory errors once RAM fills up, even when plenty of disk space is free.",
            &[
                "Re-enable it: System Properties > Advanced > Performance Settings > Advanced > Virtual memory > \"Automatically manage paging file size\".",
            ],
        )]),
    }
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::*;
    use crate::SystemInfo;
    use crate::sensors::SensorTable;

    #[test]
    fn windows_setting_checks() {
        use crate::platform::settings::WindowsSettings;
        let s = SensorTable::default();
        let run = |i: &SystemInfo| {
            let c = ctx(i, &s);
            [
                pending_restart(&c),
                memory_integrity(&c),
                gpu_scheduling(&c),
                game_mode(&c),
                pagefile(&c),
            ]
        };
        let mut info = SystemInfo::default();
        let r = run(&info);
        assert!(matches!(r[0], Outcome::Pass(_)));
        assert!(r[1..].iter().all(|o| matches!(o, Outcome::Skipped(_))));
        info.windows = WindowsSettings {
            pending_reboot: vec!["Windows Update"],
            hvci_running: Some(true),
            vbs_configured: Some(true),
            hags: Some(false),
            game_mode: Some(false),
            pagefile_disabled: Some(true),
        };
        let r = run(&info);
        let Outcome::Findings(f) = &r[1] else {
            panic!("memory integrity finding")
        };
        assert!(
            f[0].security,
            "Memory Integrity is flagged as a security setting"
        );
        let got: Vec<Vec<Level>> = r.iter().map(levels).collect();
        assert_eq!(
            got,
            vec![
                vec![Level::Info],
                vec![Level::Info],
                vec![Level::Info],
                vec![Level::Info],
                vec![Level::Warning]
            ]
        );
    }
}
