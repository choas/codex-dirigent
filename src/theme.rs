use eframe::egui::{self, Color32};

pub const CODEX_ACCENT: Color32 = Color32::from_rgb(16, 163, 127);
const CODEX_ACCENT_DARK: Color32 = Color32::from_rgb(12, 122, 95);

/// Apply the restrained Codex palette while retaining native light/dark
/// appearance selection from egui's system integration.
pub fn apply(context: &egui::Context) {
    context.set_theme(egui::ThemePreference::System);
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        context.style_mut_of(theme, |style| {
            style.spacing.item_spacing = egui::vec2(8.0, 7.0);
            style.visuals.selection.bg_fill = CODEX_ACCENT_DARK;
            style.visuals.hyperlink_color = CODEX_ACCENT;
            style.visuals.widgets.active.bg_fill = CODEX_ACCENT_DARK;
            style.visuals.widgets.hovered.bg_fill = CODEX_ACCENT;
        });
    }
}
