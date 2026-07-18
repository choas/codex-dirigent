use eframe::egui;

use crate::{PRODUCT_NAME, theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkflowStage {
    Browse,
    Cue,
    Run,
    Review,
    Commit,
}

impl WorkflowStage {
    const ALL: [Self; 5] = [
        Self::Browse,
        Self::Cue,
        Self::Run,
        Self::Review,
        Self::Commit,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Browse => "Browse",
            Self::Cue => "Cue",
            Self::Run => "Run",
            Self::Review => "Review",
            Self::Commit => "Commit",
        }
    }
}

/// Root native UI state. Domain state is added in focused modules rather than
/// accumulated directly on this type.
pub struct CodexDirigentApp {
    stage: WorkflowStage,
}

impl Default for CodexDirigentApp {
    fn default() -> Self {
        Self {
            stage: WorkflowStage::Browse,
        }
    }
}

impl eframe::App for CodexDirigentApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("toolbar").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading(PRODUCT_NAME);
                ui.add_space(16.0);
                for stage in WorkflowStage::ALL {
                    ui.selectable_value(&mut self.stage, stage, stage.label());
                }
            });
        });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            ui.vertical_centered_justified(|ui| {
                ui.add_space(120.0);
                ui.heading("Open a local Git repository");
                ui.label("Browse code, direct Codex, and review every change before committing.");
                ui.add_space(16.0);
                let open = egui::Button::new("Open Repository…")
                    .fill(theme::CODEX_ACCENT)
                    .min_size(egui::vec2(180.0, 36.0));
                let _ = ui.add(open);
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_in_browse_stage() {
        let app = CodexDirigentApp::default();
        assert_eq!(app.stage, WorkflowStage::Browse);
    }

    #[test]
    fn workflow_order_matches_review_gate() {
        let labels: Vec<_> = WorkflowStage::ALL
            .into_iter()
            .map(WorkflowStage::label)
            .collect();
        assert_eq!(labels, ["Browse", "Cue", "Run", "Review", "Commit"]);
    }
}
