use omnihw_core::{SystemInfo, format_bytes};

pub fn show(ui: &mut egui::Ui, info: &SystemInfo) {
    if let Some(t) = &info.topology {
        ui.label(egui::RichText::new("Cache instances (Windows topology)").strong());
        egui::Grid::new("cache_topo")
            .striped(true)
            .num_columns(5)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                for h in ["Cache", "Count", "Size", "Associativity", "Line"] {
                    ui.label(egui::RichText::new(h).underline());
                }
                ui.end_row();
                for s in t.cache_summary() {
                    ui.label(s.name());
                    ui.label(s.count.to_string());
                    ui.label(format_bytes(s.size_bytes));
                    ui.label(s.ways_text());
                    ui.label(format!("{} B", s.line_size));
                    ui.end_row();
                }
            });
        ui.add_space(12.0);
    }

    ui.label(egui::RichText::new("CPUID cache descriptors (current core)").strong());
    if info.cpu.caches.is_empty() {
        ui.label("Not reported by CPUID.");
        return;
    }
    egui::Grid::new("cache_cpuid")
        .striped(true)
        .num_columns(6)
        .spacing([16.0, 4.0])
        .show(ui, |ui| {
            for h in ["Cache", "Size", "Ways", "Sets", "Line", "Shared by"] {
                ui.label(egui::RichText::new(h).underline());
            }
            ui.end_row();
            for d in &info.cpu.caches {
                ui.label(d.name());
                ui.label(format_bytes(d.size_bytes));
                ui.label(d.ways.to_string());
                ui.label(d.sets.to_string());
                ui.label(format!("{} B", d.line_size));
                ui.label(format!("up to {} threads", d.shared_by));
                ui.end_row();
            }
        });
}
