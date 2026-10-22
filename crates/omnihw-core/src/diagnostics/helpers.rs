//! Shared building blocks for the checks: finding and step constructors, common fix steps
//! and text formatting.

use super::{Category, Context, Finding, Level, Outcome, Step};
use crate::bios_help::Board;
use crate::platform::Distro;

/// The distribution family, for package-manager commands in steps.
pub(super) fn distro(ctx: &Context) -> Distro {
    ctx.info.os.as_ref().map_or(Distro::Other, |o| o.distro)
}

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

/// A finding with guided steps (links to searches or websites).
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
