//! Shared building blocks for the checks: finding and step constructors, common fix steps
//! and text formatting.

use super::{Category, Context, Finding, Level, Outcome, Step};
use crate::bios_help::Board;
use crate::checkup::EVENT_CAP;
use crate::gpu::{VENDOR_AMD, VENDOR_INTEL, VENDOR_NVIDIA};

pub(super) const DDU_URL: &str = "https://www.wagnardsoft.com/display-driver-uninstaller-ddu-";
pub(super) const MBR2GPT_URL: &str = "https://learn.microsoft.com/windows/deployment/mbr-to-gpt";
const MEMTEST_URL: &str = "https://www.memtest86.com/";

/// A finding whose steps are plain text.
pub(super) fn finding(
    level: Level,
    category: Category,
    title: impl Into<String>,
    detail: impl Into<String>,
    tips: &[&str],
) -> Finding {
    guided(
        level,
        category,
        title,
        detail,
        tips.iter().map(|&t| Step::new(t)).collect(),
    )
}

/// A finding with guided steps (links to searches, Settings pages or websites).
pub(super) fn guided(
    level: Level,
    category: Category,
    title: impl Into<String>,
    detail: impl Into<String>,
    steps: Vec<Step>,
) -> Finding {
    Finding {
        level,
        category,
        title: title.into(),
        detail: detail.into(),
        steps,
        security: false,
        bios: false,
        fix: None,
    }
}

/// The usual outcome of a per-device check: its findings if any, otherwise a pass listing
/// what was examined (`seen`), or `skipped` when nothing could be examined.
pub(super) fn summarize(out: Vec<Finding>, seen: &[String], skipped: impl Into<String>) -> Outcome {
    if !out.is_empty() {
        Outcome::Findings(out)
    } else if seen.is_empty() {
        Outcome::Skipped(skipped.into())
    } else {
        Outcome::Pass(seen.join("; "))
    }
}

/// "1 device", "3 devices".
pub(super) fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// "200 or more" when a capped event list is full.
pub(super) fn count_text(n: usize) -> String {
    if n >= EVENT_CAP {
        format!("{n} or more")
    } else {
        n.to_string()
    }
}

/// "2026-10-01".
pub(super) fn format_date((y, m, d): (i64, u32, u32)) -> String {
    format!("{y}-{m:02}-{d:02}")
}

/// Whole calendar months from (`year`, `month`) to today.
pub(super) fn months_since(ctx: &Context, year: i64, month: u32) -> i64 {
    let (ty, tm, _) = ctx.today;
    (ty - year) * 12 + i64::from(tm) - i64::from(month)
}

/// A search query about this PC's motherboard: "<board> <what>", or `generic` when the board
/// isn't known.
pub(super) fn board_query(ctx: &Context, what: &str, generic: &str) -> String {
    let name = Board::detect(ctx.info).search_name();
    if name.is_empty() {
        generic.to_string()
    } else {
        format!("{name} {what}")
    }
}

/// Step with a Google search for this board's BIOS downloads.
pub(super) fn bios_update_step(ctx: &Context, text: &str) -> Step {
    Step::search(
        text,
        board_query(ctx, "BIOS download", "motherboard BIOS update download"),
    )
}

/// Search step for the board vendor's driver downloads.
pub(super) fn board_drivers_step(ctx: &Context, text: &str) -> Step {
    Step::search(
        text,
        board_query(
            ctx,
            "drivers download",
            "motherboard chipset drivers download",
        ),
    )
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

/// Run MemTest86 for a thorough memory test.
pub(super) fn memtest86_step() -> Step {
    Step::url(
        "For a more thorough test, run MemTest86 from a USB stick for a few passes.",
        MEMTEST_URL,
    )
}

/// Windows Memory Diagnostic, then MemTest86.
pub(super) fn memory_test_steps() -> [Step; 2] {
    [
        Step::new(
            "Test the memory: press Start, type \"Windows Memory Diagnostic\" and choose Restart now and check for problems.",
        ),
        memtest86_step(),
    ]
}
