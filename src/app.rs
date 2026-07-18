use eframe::egui;
use std::path::PathBuf;

use crate::{
    PRODUCT_NAME,
    cue::{Cue, CueTarget},
    review::{Session, SessionState},
    theme,
    workspace::Workspace,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkflowStage {
    Browse,
    Cue,
    Run,
    Review,
    Commit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CueScope {
    Repository,
    File,
    Lines,
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
    cue_scope: CueScope,
    cue_text: String,
    line_start: usize,
    line_end: usize,
    session: Option<Session>,
    commit_message: String,
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
            cue_scope: CueScope::Repository,
            cue_text: String::new(),
            line_start: 1,
            line_end: 1,
            session: None,
            commit_message: String::new(),
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
            if self.stage == WorkflowStage::Cue {
                self.cue_ui(ui);
            } else if matches!(self.stage, WorkflowStage::Review | WorkflowStage::Commit) {
                self.review_ui(ui);
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

    fn cue_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Create a cue");
        ui.label("Codex receives this instruction with the selected repository context.");
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.cue_scope, CueScope::Repository, "Repository");
            ui.add_enabled_ui(self.selected_file.is_some(), |ui| {
                ui.selectable_value(&mut self.cue_scope, CueScope::File, "File");
                ui.selectable_value(&mut self.cue_scope, CueScope::Lines, "Line range");
            });
        });
        if matches!(self.cue_scope, CueScope::File | CueScope::Lines) {
            ui.label(self.selected_file.as_ref().map_or_else(
                || "Select a file first".to_owned(),
                |path| path.display().to_string(),
            ));
        }
        if self.cue_scope == CueScope::Lines {
            ui.horizontal(|ui| {
                ui.label("Lines");
                ui.add(egui::DragValue::new(&mut self.line_start).range(1..=usize::MAX));
                ui.label("through");
                ui.add(egui::DragValue::new(&mut self.line_end).range(1..=usize::MAX));
            });
        }
        ui.add(
            egui::TextEdit::multiline(&mut self.cue_text)
                .hint_text("Describe the change you want Codex to make…")
                .desired_rows(8),
        );
        if ui.button("Create Cue").clicked() {
            let target = match self.cue_scope {
                CueScope::Repository => Some(CueTarget::Repository),
                CueScope::File => self.selected_file.clone().map(CueTarget::File),
                CueScope::Lines => self.selected_file.clone().map(|path| CueTarget::Lines {
                    path,
                    start: self.line_start,
                    end: self.line_end,
                }),
            };
            match target.map(|target| Cue::new(self.cue_text.clone(), target)) {
                Some(Ok(cue)) => {
                    self.session = Some(Session::new(cue));
                    self.error = None;
                    self.stage = WorkflowStage::Run;
                }
                Some(Err(error)) => self.error = Some(error.to_string()),
                None => self.error = Some("select a file for this cue".to_owned()),
            }
        }
    }

    fn review_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Review changes");
        if let Some(session) = &self.session {
            ui.label(format!("State: {:?}", session.state()));
        }
        ui.separator();
        egui::ScrollArea::both().max_height(420.0).show(ui, |ui| {
            if self.diff_text.is_empty() {
                ui.label("No changes to review.");
            } else {
                ui.add(
                    egui::TextEdit::multiline(&mut self.diff_text)
                        .font(egui::TextStyle::Monospace)
                        .interactive(false)
                        .desired_width(f32::INFINITY),
                );
            }
        });
        ui.separator();

        let reviewing = self
            .session
            .as_ref()
            .is_some_and(|session| matches!(session.state(), SessionState::Reviewing { .. }));
        let accepted = self
            .session
            .as_ref()
            .is_some_and(|session| session.state() == &SessionState::Accepted);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(reviewing, egui::Button::new("Accept Reviewed Diff"))
                .clicked()
                && let Some(session) = &mut self.session
                && let Err(error) = session.accept(&self.diff_text)
            {
                self.error = Some(error.to_string());
            }
            if ui
                .add_enabled(reviewing || accepted, egui::Button::new("Reject Changes"))
                .clicked()
            {
                let result = self
                    .workspace
                    .as_mut()
                    .ok_or_else(|| "no repository is open".to_owned())
                    .and_then(|workspace| {
                        workspace.reject_run_changes().map_err(|e| e.to_string())
                    });
                match result {
                    Ok(()) => {
                        if let Some(session) = &mut self.session {
                            let _ = session.reject();
                        }
                        self.refresh();
                    }
                    Err(error) => self.error = Some(error),
                }
            }
        });

        if accepted {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label("Commit message");
                ui.text_edit_singleline(&mut self.commit_message);
                if ui.button("Commit Accepted Changes").clicked() {
                    self.commit_accepted();
                }
            });
        }
    }

    fn commit_accepted(&mut self) {
        let approval = self.session.as_ref().and_then(Session::approval).cloned();
        let Some(approval) = approval else {
            self.error = Some("accept the reviewed diff before committing".to_owned());
            return;
        };
        let result = self
            .workspace
            .as_mut()
            .ok_or_else(|| "no repository is open".to_owned())
            .and_then(|workspace| {
                workspace
                    .commit_approved(&approval, &self.commit_message)
                    .map_err(|error| error.to_string())
            });
        match result {
            Ok(commit) => {
                if let Some(session) = &mut self.session {
                    let _ = session.mark_committed(commit);
                }
                self.refresh();
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
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
