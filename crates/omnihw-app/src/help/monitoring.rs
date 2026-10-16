//! Help pages: recording (compare, bottleneck), alerts and reports.

use super::{bullets, details, h, p};
use egui::Ui;
use omnihw_core::bottleneck as bn;

pub(super) fn recording(ui: &mut Ui) {
    h(ui, "Recording a session");
    bullets(
        ui,
        &[
            "Click Record in the tab bar (or Start recording on the Recording tab). Every sensor is recorded at the sampling interval until you click Stop.",
            r"Each recording is saved as a CSV file in your Documents folder named OmniHW-Recording-<date-time>.csv (opens in Excel). It is written as it records, so a crash loses at most one row.",
            "Sensors that only appear after recording starts aren't added to it.",
        ],
    );
    h(ui, "Session mode");
    p(
        ui,
        "Shows the current or last recording. Tick sensors on the left to chart them; sensors with the same unit share a chart. \
         Hover a chart to read exact values. Under each chart, a table lists each sensor's first, last, change, minimum, average and maximum. \
         Show CSV opens the folder with the file selected.",
    );
    h(ui, "Compare mode");
    bullets(
        ui,
        &[
            "Pick recording A and B: the one in memory or any saved recording in Documents (Refresh list rescans the folder).",
            "Charts overlay both sessions on elapsed time: A solid, B dashed. Only sensors present in both are offered.",
            "The table shows average and maximum for A and B and the difference; for temperatures and power, lower is shown in green.",
            "Typical uses: before vs after a repaste, a driver update, enabling XMP, or a new fan curve. Record the same game or test for the same length both times.",
        ],
    );
    h(ui, "Bottleneck mode");
    p(
        ui,
        "Tells you what limited performance during a recording: the graphics card, the CPU, or neither. Record a few minutes \
         of gameplay, stop the recording, then pick it here.",
    );
    bullets(
        ui,
        &[
            format!("GPU-bound: GPU load at {:.0} % or more. Normal when playing at high settings or resolution.", bn::GPU_BOUND),
            format!(
                "CPU-bound: GPU load below {:.0} % while the busiest CPU thread is at {:.0} % or more (or total CPU at {:.0} % or more). Games often wait on a single thread even when total CPU usage looks low.",
                bn::GPU_HEADROOM,
                bn::THREAD_BUSY,
                bn::CPU_TOTAL_BUSY
            ),
            "Not limited: neither is maxed out, usually because of a frame cap or V-Sync. That's fine.".into(),
            format!("Idle moments (GPU and CPU both below {:.0} %) are left out.", bn::IDLE),
            "The verdict is the situation that covers at least half of the active time; otherwise it's Mixed.".into(),
            format!(
                "With fewer than {} active samples the result is marked as low confidence.",
                bn::MIN_ACTIVE
            ),
        ],
    );
    p(
        ui,
        "Below the verdict you'll find a chart of GPU load and the busiest CPU thread with a colored strip showing the verdict \
         at each moment, contributing factors (VRAM or RAM nearly full, a single overloaded thread) and tips.",
    );
    details(
        ui,
        "rec",
        &[
            "The analysis uses load levels, not frame times, so frame caps are inferred.",
            "Recordings keep all samples in memory while they run (about 1 MB per hour at 1 s); CSVs are about 2 MB per hour.",
            "Saved recordings are matched in Compare by sensor group and name.",
        ],
    );
}

pub(super) fn alerts(ui: &mut Ui) {
    h(ui, "What it does");
    p(
        ui,
        "Alerts watch a sensor and notify you when it stays above or below a limit for a while, e.g. \"GPU temperature over 85 °C for 30 seconds\". \
         They're checked on every sample while Omni-HW is running, whichever tab is open.",
    );
    h(ui, "Setting alerts up");
    bullets(
        ui,
        &[
            "Quick add offers common alerts for the sensors your PC has: GPU over 85 °C, CPU usage over 95 % for 2 minutes, memory over 90 % for 1 minute, each drive over 70 °C, and battery below 20 % on laptops.",
            "Add custom alert creates a rule you can point at any sensor: choose above or below, the limit, and how many seconds it must hold (0 = immediately).",
            "Untick On to pause a rule without deleting it; Remove deletes it.",
            "Test notification shows a sample notification so you can check Windows displays them.",
        ],
    );
    h(ui, "When an alert fires");
    bullets(
        ui,
        &[
            "You get a Windows notification and the taskbar button flashes if Omni-HW isn't in front.",
            "A red ALERT button appears in the tab bar; click it to open the Alerts tab.",
            "Each alert fires once, then re-arms when the reading goes back past the limit.",
            "History lists the alerts that fired since Omni-HW started.",
        ],
    );
    details(
        ui,
        "alerts",
        &[
            r"Rules are saved automatically to %APPDATA%\OmniHW\alerts.tsv.",
            "Windows hides notifications while Do Not Disturb / Focus is on or a full-screen game is running; the ALERT button and History still record them.",
        ],
    );
}

pub(super) fn reports(ui: &mut Ui) {
    h(ui, "Save report");
    p(
        ui,
        "Writes a complete report as a web page (HTML) to your Documents folder and opens it in your browser. It starts with \
         the Health Check results, then the system, processor, mainboard, memory, every graphics card and drive, and a table \
         of all sensors with current, minimum, maximum and average. It works in light and dark mode and is easy to attach \
         to a forum post or support ticket.",
    );
    h(ui, "Copy");
    p(
        ui,
        "Copies the same report as plain text to the clipboard, ready to paste into a chat or forum.",
    );
    details(
        ui,
        "reports",
        &[
            "Reports are named OmniHW-Report-<date-time>.html (UTC time).",
            "Gaming findings are tagged (Gaming) and security-related ones carry a caution note, as in the app.",
            "Reports include serial numbers of drives; remove them before posting publicly if you prefer.",
        ],
    );
}
