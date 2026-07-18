use eframe::egui::{self, Color32};

pub const CODEX_ACCENT: Color32 = Color32::from_rgb(16, 163, 127);
const CODEX_ACCENT_DARK: Color32 = Color32::from_rgb(12, 122, 95);

/// Apply the restrained Codex palette while retaining native light/dark
/// appearance selection from egui's system integration.
pub fn apply(context: &egui::Context) {
    let mut visuals = context.global_style().visuals.clone();
    visuals.selection.bg_fill = CODEX_ACCENT_DARK;
    visuals.hyperlink_color = CODEX_ACCENT;
    visuals.widgets.active.bg_fill = CODEX_ACCENT_DARK;
    visuals.widgets.hovered.bg_fill = CODEX_ACCENT;
    context.set_visuals(visuals);
}
