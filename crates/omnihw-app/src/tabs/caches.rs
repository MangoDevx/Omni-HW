//! Caches page: cache instances from the Windows topology and the CPUID descriptors.

use crate::app::OmniApp;
use crate::ui::{self, Table};
use gpui_kit::{AnyElement, Context, IntoElement, ParentElement, Window};
use omnihw_core::format_bytes;

pub fn show(app: &mut OmniApp, _window: &mut Window, cx: &mut Context<OmniApp>) -> AnyElement {
    let Some(info) = app.sampler.info() else {
        return ui::column().into_any_element();
    };
    let mut page = ui::column();
    if let Some(t) = &info.topology {
        let mut table = Table::new(&["Cache", "Count", "Size", "Associativity", "Line"]).striped();
        for s in t.cache_summary() {
            table = table.text_row([
                s.name(),
                s.count.to_string(),
                format_bytes(s.size_bytes),
                s.ways_text(),
                format!("{} B", s.line_size),
            ]);
        }
        page = page
            .child(ui::strong("Cache instances (Windows topology)"))
            .child(table.build(cx))
            .child(ui::space(12.0));
    }

    page = page.child(ui::strong("CPUID cache descriptors (current core)"));
    if info.cpu.caches.is_empty() {
        return page
            .child(ui::label("Not reported by CPUID."))
            .into_any_element();
    }
    let mut table = Table::new(&["Cache", "Size", "Ways", "Sets", "Line", "Shared by"]).striped();
    for d in &info.cpu.caches {
        table = table.text_row([
            d.name(),
            format_bytes(d.size_bytes),
            d.ways.to_string(),
            d.sets.to_string(),
            format!("{} B", d.line_size),
            format!("up to {} threads", d.shared_by),
        ]);
    }
    page.child(table.build(cx)).into_any_element()
}
