use eframe::egui;
use std::path::PathBuf;

use crate::{PRODUCT_NAME, theme, workspace::Workspace};

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
    workspace: Option<Workspace>,
    selected_file: Option<PathBuf>,
    file_text: String,
    diff_text: String,
    error: Option<String>,
}

impl Default for CodexDirigentApp {
    fn default() -> Self {
        Self {
            stage: WorkflowStage::Browse,
            workspace: None,
            selected_file: None,
            file_text: String::new(),
            diff_text: String::new(),
            error: None,
        }
    }
}

impl CodexDirigentApp {
    fn choose_repository(&mut self) {
        if let Some(path) = rfd::FileDialog::new().pick_folder() {
            self.open_repository(&path);
        }
    }

    fn open_repository(&mut self, path: &std::path::Path) {
        match Workspace::open(path) {
            Ok(workspace) => {
                self.diff_text = workspace.working_diff().unwrap_or_default();
                self.workspace = Some(workspace);
                self.selected_file = None;
                self.file_text.clear();
                self.error = None;
                self.stage = WorkflowStage::Browse;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn select_file(&mut self, path: PathBuf) {
        let Some(workspace) = &self.workspace else {
            return;
        };
        match workspace.read_text(&path) {
            Ok(text) => {
                self.selected_file = Some(path);
                self.file_text = text;
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn refresh(&mut self) {
        let Some(workspace) = &mut self.workspace else {
            return;
        };
        if let Err(error) = workspace.refresh() {
            self.error = Some(error.to_string());
            return;
        }
        self.diff_text = workspace.working_diff().unwrap_or_default();
        if let Some(path) = self.selected_file.clone() {
            self.select_file(path);
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(PRODUCT_NAME);
            ui.add_space(16.0);
            for stage in WorkflowStage::ALL {
                ui.selectable_value(&mut self.stage, stage, stage.label());
            }
            ui.separator();
            if ui
                .button("Open…")
                .on_hover_text("Open Repository (⌘O)")
                .clicked()
            {
                self.choose_repository();
            }
            if ui.button("Refresh").on_hover_text("Refresh (⌘R)").clicked() {
                self.refresh();
            }
            if let Some(workspace) = &self.workspace {
                ui.separator();
                ui.label(format!(
                    "{} · {}",
                    workspace.branch(),
                    workspace.root().display()
                ));
            }
        });
        if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
    }

    fn workspace_ui(&mut self, ui: &mut egui::Ui) {
        let files = self
            .workspace
            .as_ref()
            .map(|workspace| workspace.files().to_vec())
            .unwrap_or_default();
        egui::Panel::left("file_tree")
            .resizable(true)
            .default_size(260.0)
            .show_inside(ui, |ui| {
                ui.heading("Files");
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for file in files {
                        let marker = file
                            .status
                            .map_or("  ".to_owned(), |status| format!("{status} "));
                        let label = format!("{marker}{}", file.relative_path.display());
                        let selected = self.selected_file.as_ref() == Some(&file.relative_path);
                        if ui.selectable_label(selected, label).clicked() {
                            self.select_file(file.relative_path);
                        }
                    }
                });
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            if self.stage == WorkflowStage::Review {
                ui.heading("Working tree diff");
                ui.separator();
                if self.diff_text.is_empty() {
                    ui.label("No changes to review.");
                } else {
                    egui::ScrollArea::both().show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.diff_text)
                                .font(egui::TextStyle::Monospace)
                                .interactive(false)
                                .desired_width(f32::INFINITY),
                        );
                    });
                }
            } else if let Some(path) = &self.selected_file {
                ui.heading(path.display().to_string());
                ui.separator();
                egui::ScrollArea::both().show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.file_text)
                            .font(egui::TextStyle::Monospace)
                            .interactive(false)
                            .desired_width(f32::INFINITY),
                    );
                });
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("Select a file to view it read-only.");
                });
            }
        });
    }

    fn shortcuts(&mut self, context: &egui::Context) {
        if context.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) {
            self.choose_repository();
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::R)) {
            self.refresh();
        }
    }
}

impl eframe::App for CodexDirigentApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.shortcuts(ui.ctx());
        egui::Panel::top("toolbar").show_inside(ui, |ui| {
            self.toolbar(ui);
        });

        if self.workspace.is_some() {
            self.workspace_ui(ui);
        } else {
            egui::CentralPanel::default().show_inside(ui, |ui| {
                ui.vertical_centered_justified(|ui| {
                    ui.add_space(120.0);
                    ui.heading("Open a local Git repository");
                    ui.label(
                        "Browse code, direct Codex, and review every change before committing.",
                    );
                    ui.add_space(16.0);
                    let open = egui::Button::new("Open Repository…")
                        .fill(theme::CODEX_ACCENT)
                        .min_size(egui::vec2(180.0, 36.0));
                    if ui.add(open).clicked() {
                        self.choose_repository();
                    }
                });
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_in_browse_stage() {
        let app = CodexDirigentApp::default();
        assert_eq!(app.stage, WorkflowStage::Browse);
        assert!(app.workspace.is_none());
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
