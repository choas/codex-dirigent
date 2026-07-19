use eframe::egui;
use std::fmt::Write as _;
use std::path::PathBuf;

use crate::{
    PRODUCT_NAME,
    codex::{self, CodexEvent, CodexRun},
    cue::{Cue, CueTarget},
    review::{Session, SessionState},
    settings::{self, Settings},
    theme,
    workspace::{CueWorktree, Workspace},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppView {
    Browse,
    Board,
    NewCue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CueScope {
    Repository,
    File,
    Lines,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CueLane {
    Inbox,
    Run,
    Review,
    Done,
    Archive,
}

impl CueLane {
    const ALL: [Self; 5] = [
        Self::Inbox,
        Self::Run,
        Self::Review,
        Self::Done,
        Self::Archive,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Inbox => "Inbox",
            Self::Run => "Run",
            Self::Review => "Review",
            Self::Done => "Done",
            Self::Archive => "Archive",
        }
    }
}

struct CueCard {
    id: u64,
    session: Session,
    lane: CueLane,
    worktree: Option<CueWorktree>,
    active_run: Option<CodexRun>,
    active_run_id: Option<u64>,
    log: Vec<String>,
    follow_up: String,
    commit_message: String,
    branch_commit: Option<String>,
    error: Option<String>,
}

impl CueCard {
    fn new(id: u64, cue: Cue) -> Self {
        Self {
            id,
            session: Session::new(cue),
            lane: CueLane::Inbox,
            worktree: None,
            active_run: None,
            active_run_id: None,
            log: Vec::new(),
            follow_up: String::new(),
            commit_message: String::new(),
            branch_commit: None,
            error: None,
        }
    }

    fn is_running(&self) -> bool {
        self.active_run.is_some()
    }
}

#[derive(Debug, Clone, Copy)]
enum CardAction {
    Select(u64),
    Run(u64),
    Cancel(u64),
    Archive(u64),
}

/// Root native UI state with independent cue worktrees.
pub struct CodexDirigentApp {
    view: AppView,
    workspace: Option<Workspace>,
    selected_file: Option<PathBuf>,
    file_text: String,
    error: Option<String>,
    cue_scope: CueScope,
    cue_text: String,
    line_start: usize,
    line_end: usize,
    cues: Vec<CueCard>,
    next_cue_id: u64,
    selected_cue: Option<u64>,
    confirm_reject: Option<u64>,
    settings: Settings,
    settings_path: Option<PathBuf>,
    settings_open: bool,
}

impl Default for CodexDirigentApp {
    fn default() -> Self {
        Self {
            view: AppView::Browse,
            workspace: None,
            selected_file: None,
            file_text: String::new(),
            error: None,
            cue_scope: CueScope::Repository,
            cue_text: String::new(),
            line_start: 1,
            line_end: 1,
            cues: Vec::new(),
            next_cue_id: 1,
            selected_cue: None,
            confirm_reject: None,
            settings: Settings::default(),
            settings_path: None,
            settings_open: false,
        }
    }
}

impl CodexDirigentApp {
    #[must_use]
    pub fn load() -> Self {
        let path = settings::default_path().ok();
        let (loaded_settings, warning) = path.as_ref().map_or_else(
            || {
                (
                    Settings::default(),
                    Some("settings location is unavailable; changes will not persist".to_owned()),
                )
            },
            |settings_path| match settings::load(settings_path) {
                Ok(settings) => (settings, None),
                Err(error) => (
                    Settings::default(),
                    Some(format!("{error}; using defaults")),
                ),
            },
        );
        let recent = loaded_settings.last_repository.clone();
        let mut app = Self {
            settings: loaded_settings,
            settings_path: path,
            error: warning,
            ..Self::default()
        };
        if let Some(repository) = recent
            && repository.exists()
        {
            app.open_repository(&repository);
        }
        app
    }

    fn any_running(&self) -> bool {
        self.cues.iter().any(CueCard::is_running)
    }

    fn choose_repository(&mut self) {
        if self.any_running() {
            self.error =
                Some("cancel all active cue runs before opening another repository".to_owned());
            return;
        }
        if self.cues.iter().any(|cue| cue.lane != CueLane::Archive) {
            self.error = Some("archive active cues before opening another repository".to_owned());
            return;
        }
        if let Some(path) = rfd::FileDialog::new().pick_folder() {
            self.open_repository(&path);
        }
    }

    fn open_repository(&mut self, path: &std::path::Path) {
        match Workspace::open(path) {
            Ok(workspace) => {
                self.settings.last_repository = Some(workspace.root().to_path_buf());
                self.workspace = Some(workspace);
                self.selected_file = None;
                self.file_text.clear();
                self.cues.clear();
                self.selected_cue = None;
                self.error = None;
                self.view = AppView::Browse;
                self.save_settings();
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
        if let Some(path) = self.selected_file.clone() {
            self.select_file(path);
        }
    }

    fn select_file(&mut self, path: PathBuf) {
        let Some(workspace) = &self.workspace else {
            return;
        };
        match workspace.read_text(&path) {
            Ok(text) => {
                self.selected_file = Some(path);
                self.file_text = with_line_numbers(&text);
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(PRODUCT_NAME);
            ui.add_space(16.0);
            ui.selectable_value(&mut self.view, AppView::Browse, "Browse");
            ui.add_enabled_ui(self.workspace.is_some(), |ui| {
                ui.selectable_value(&mut self.view, AppView::Board, "Cue Board");
                ui.selectable_value(&mut self.view, AppView::NewCue, "New Cue");
            });
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
            if ui.button("Settings…").clicked() {
                self.settings_open = true;
            }
        });
        if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(workspace) = &self.workspace {
                ui.strong(workspace.branch());
                ui.separator();
                ui.label(workspace.root().display().to_string());
                let running = self.cues.iter().filter(|cue| cue.is_running()).count();
                if running > 0 {
                    ui.separator();
                    ui.colored_label(
                        theme::CODEX_ACCENT,
                        format!("{running} Codex run(s) active"),
                    );
                }
            } else {
                ui.label("No repository open");
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label("⌘O Open  ·  ⌘R Refresh  ·  ⌘, Settings");
            });
        });
    }

    fn workspace_ui(&mut self, ui: &mut egui::Ui) {
        let files = self
            .workspace
            .as_ref()
            .map(|workspace| workspace.files().to_vec())
            .unwrap_or_default();
        egui::Panel::left("file_tree")
            .resizable(true)
            .default_size(240.0)
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

        egui::CentralPanel::default().show_inside(ui, |ui| match self.view {
            AppView::Browse => self.browser_ui(ui),
            AppView::Board => self.board_ui(ui),
            AppView::NewCue => self.new_cue_ui(ui),
        });
    }

    fn browser_ui(&mut self, ui: &mut egui::Ui) {
        if let Some(path) = &self.selected_file {
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
    }

    fn new_cue_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Add a cue to the Inbox");
        ui.label("Queue as many cues as needed, then run them together or one at a time.");
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
        if ui.button("Add Cue to Inbox").clicked() {
            self.create_cue();
        }
    }

    fn create_cue(&mut self) {
        let target = match self.cue_scope {
            CueScope::Repository => Some(CueTarget::Repository),
            CueScope::File => self.selected_file.clone().map(CueTarget::File),
            CueScope::Lines => self.selected_file.clone().map(|path| CueTarget::Lines {
                path,
                start: self.line_start,
                end: self.line_end,
            }),
        };
        let cue = match target.map(|target| Cue::new(self.cue_text.clone(), target)) {
            Some(Ok(cue)) => cue,
            Some(Err(error)) => {
                self.error = Some(error.to_string());
                return;
            }
            None => {
                self.error = Some("select a file for this cue".to_owned());
                return;
            }
        };
        if self.workspace.is_none() {
            return;
        }
        let id = self.next_cue_id;
        self.next_cue_id += 1;
        self.cues.push(CueCard::new(id, cue));
        self.cue_text.clear();
        self.error = None;
        self.view = AppView::Board;
    }

    fn board_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Cue Board");
            if ui.button("+ New Cue").clicked() {
                self.view = AppView::NewCue;
            }
            let inbox_count = self
                .cues
                .iter()
                .filter(|cue| cue.lane == CueLane::Inbox)
                .count();
            if ui
                .add_enabled(
                    inbox_count > 0,
                    egui::Button::new(format!("Run Inbox ({inbox_count})"))
                        .fill(theme::CODEX_ACCENT),
                )
                .clicked()
            {
                self.run_inbox();
            }
        });
        ui.label(
            "Inbox cues create isolated worktrees only when started. Review and merge each result independently.",
        );
        ui.separator();

        let mut action = None;
        egui::ScrollArea::horizontal()
            .id_salt("cue_lanes")
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    for lane in CueLane::ALL {
                        let count = self.cues.iter().filter(|cue| cue.lane == lane).count();
                        ui.group(|ui| {
                            ui.set_min_width(245.0);
                            ui.set_max_width(280.0);
                            ui.heading(format!("{}  {count}", lane.label()));
                            ui.separator();
                            for cue in self.cues.iter_mut().filter(|cue| cue.lane == lane) {
                                if let Some(next) = render_card(ui, cue) {
                                    action = Some(next);
                                }
                                ui.add_space(6.0);
                            }
                        });
                    }
                });
            });
        if let Some(action) = action {
            self.handle_card_action(action);
        }

        if self.selected_cue.is_some() {
            ui.separator();
            self.selected_cue_ui(ui);
        }
    }

    fn handle_card_action(&mut self, action: CardAction) {
        match action {
            CardAction::Select(id) => self.selected_cue = Some(id),
            CardAction::Run(id) => self.start_initial_run(id),
            CardAction::Cancel(id) => {
                if let Some(cue) = self.cues.iter().find(|cue| cue.id == id)
                    && let Some(run) = &cue.active_run
                {
                    run.cancel();
                }
            }
            CardAction::Archive(id) => self.archive_cue(id),
        }
    }

    fn selected_cue_ui(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.selected_cue else {
            return;
        };
        let Some(index) = self.cues.iter().position(|cue| cue.id == id) else {
            self.selected_cue = None;
            return;
        };
        let mut send_follow_up = false;
        let mut accept = false;
        let mut commit_merge = false;
        let mut reject = false;
        {
            let cue = &mut self.cues[index];
            ui.heading(format!(
                "Cue #{} · {}",
                cue.id,
                cue.session.cue().instruction()
            ));
            if let Some(worktree) = &cue.worktree {
                ui.label(format!("Branch: {}", worktree.branch()));
            } else {
                ui.label("Queued in Inbox; no worktree created yet.");
            }
            if let Some(error) = &cue.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            if cue.lane == CueLane::Review {
                ui.label("Reviewed worktree diff");
                egui::ScrollArea::both().max_height(320.0).show(ui, |ui| {
                    let mut diff = cue.session.review_diff().to_owned();
                    ui.add(
                        egui::TextEdit::multiline(&mut diff)
                            .font(egui::TextStyle::Monospace)
                            .interactive(false)
                            .desired_width(f32::INFINITY),
                    );
                });
                let reviewing = matches!(cue.session.state(), SessionState::Reviewing { .. });
                let accepted = cue.session.state() == &SessionState::Accepted;
                if reviewing {
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut cue.follow_up);
                        send_follow_up = ui.button("Send Follow-up").clicked();
                    });
                }
                ui.horizontal(|ui| {
                    accept = ui
                        .add_enabled(reviewing, egui::Button::new("Accept Diff"))
                        .clicked();
                    reject = ui
                        .add_enabled(reviewing || accepted, egui::Button::new("Reject Cue"))
                        .clicked();
                });
                ui.horizontal(|ui| {
                    ui.label("Commit message");
                    ui.text_edit_singleline(&mut cue.commit_message);
                    commit_merge = ui
                        .add_enabled(
                            accepted,
                            egui::Button::new("Commit & Merge to Main").fill(theme::CODEX_ACCENT),
                        )
                        .clicked();
                });
            } else {
                ui.label(format!("Status: {}", cue.lane.label()));
            }
            if !cue.log.is_empty() {
                ui.collapsing("Codex progress", |ui| {
                    for line in &cue.log {
                        ui.label(line);
                    }
                });
            }
        }
        if send_follow_up {
            self.start_follow_up(id);
        }
        if accept {
            self.accept_cue(id);
        }
        if commit_merge {
            self.commit_and_merge(id);
        }
        if reject {
            self.confirm_reject = Some(id);
        }
    }

    fn start_initial_run(&mut self, id: u64) {
        let Some(index) = self.cues.iter().position(|cue| cue.id == id) else {
            return;
        };
        if self.cues[index].worktree.is_none() {
            let Some(workspace) = &self.workspace else {
                return;
            };
            match workspace.create_cue_worktree(id) {
                Ok(worktree) => {
                    self.cues[index].worktree = Some(worktree);
                    self.cues[index].lane = CueLane::Run;
                    self.cues[index].error = None;
                }
                Err(error) => {
                    self.cues[index].error = Some(error.to_string());
                    return;
                }
            }
        }
        let prompt = self.cues[index].session.cue().prompt();
        match self.cues[index].session.begin_run() {
            Ok(run_id) => self.spawn_codex(index, run_id, prompt),
            Err(error) => self.cues[index].error = Some(error.to_string()),
        }
    }

    fn run_inbox(&mut self) {
        let queued: Vec<_> = self
            .cues
            .iter()
            .filter(|cue| cue.lane == CueLane::Inbox)
            .map(|cue| cue.id)
            .collect();
        for id in queued {
            self.start_initial_run(id);
        }
    }

    fn start_follow_up(&mut self, id: u64) {
        let Some(index) = self.cues.iter().position(|cue| cue.id == id) else {
            return;
        };
        let instruction = self.cues[index].follow_up.clone();
        match self.cues[index].session.follow_up(instruction) {
            Ok(run_id) => {
                let prompt = codex::follow_up_prompt(
                    self.cues[index].session.cue(),
                    self.cues[index].session.messages(),
                );
                self.cues[index].follow_up.clear();
                self.cues[index].lane = CueLane::Run;
                self.spawn_codex(index, run_id, prompt);
            }
            Err(error) => self.cues[index].error = Some(error.to_string()),
        }
    }

    fn spawn_codex(&mut self, index: usize, run_id: u64, prompt: String) {
        let Some(repository) = self.cues[index]
            .worktree
            .as_ref()
            .map(|worktree| worktree.path().to_path_buf())
        else {
            let cue = &mut self.cues[index];
            let message = "cue worktree was not created".to_owned();
            let _ = cue.session.execution_failed(run_id, message.clone());
            cue.lane = CueLane::Inbox;
            cue.error = Some(message);
            return;
        };
        match codex::start(&repository, prompt, self.settings.codex_config()) {
            Ok(run) => {
                let cue = &mut self.cues[index];
                cue.active_run = Some(run);
                cue.active_run_id = Some(run_id);
                cue.log.clear();
                cue.error = None;
            }
            Err(error) => {
                let cue = &mut self.cues[index];
                let _ = cue.session.execution_failed(run_id, error.to_string());
                cue.lane = if matches!(cue.session.state(), SessionState::Reviewing { .. }) {
                    CueLane::Review
                } else {
                    CueLane::Run
                };
                cue.error = Some(error.to_string());
            }
        }
    }

    fn poll_codex(&mut self, context: &egui::Context) {
        let mut events = Vec::new();
        for (index, cue) in self.cues.iter().enumerate() {
            if let Some(run) = &cue.active_run {
                while let Ok(event) = run.try_recv() {
                    events.push((index, event));
                }
            }
        }
        for (index, event) in events {
            match event {
                CodexEvent::Progress(message) => {
                    let cue = &mut self.cues[index];
                    cue.log.push(message);
                    if cue.log.len() > 500 {
                        cue.log.remove(0);
                    }
                }
                CodexEvent::Completed { summary } => {
                    let cue = &mut self.cues[index];
                    let run_id = cue.active_run_id.take();
                    cue.active_run = None;
                    let diff = cue.worktree.as_ref().map_or_else(
                        || {
                            Err(crate::workspace::WorkspaceError::Git(
                                "cue worktree was not created".to_owned(),
                            ))
                        },
                        |worktree| {
                            worktree
                                .open()
                                .and_then(|workspace| workspace.working_diff())
                        },
                    );
                    match (run_id, diff) {
                        (Some(run_id), Ok(diff)) => {
                            if let Err(error) = cue.session.finish_run(run_id, summary, diff) {
                                cue.error = Some(error.to_string());
                            } else {
                                cue.lane = CueLane::Review;
                                self.selected_cue = Some(cue.id);
                            }
                        }
                        (Some(run_id), Err(error)) => {
                            let _ = cue.session.execution_failed(run_id, error.to_string());
                            cue.lane =
                                if matches!(cue.session.state(), SessionState::Reviewing { .. }) {
                                    CueLane::Review
                                } else {
                                    CueLane::Run
                                };
                            cue.error = Some(error.to_string());
                        }
                        (None, _) => cue.error = Some("run identifier was lost".to_owned()),
                    }
                }
                CodexEvent::Cancelled | CodexEvent::Failed(_) => {
                    let message = match event {
                        CodexEvent::Cancelled => "Codex run cancelled".to_owned(),
                        CodexEvent::Failed(message) => message,
                        _ => unreachable!(),
                    };
                    let cue = &mut self.cues[index];
                    let run_id = cue.active_run_id.take();
                    cue.active_run = None;
                    if let Some(run_id) = run_id {
                        let _ = cue.session.execution_failed(run_id, message.clone());
                    }
                    cue.lane = if matches!(cue.session.state(), SessionState::Reviewing { .. }) {
                        CueLane::Review
                    } else {
                        CueLane::Run
                    };
                    cue.error = Some(message);
                }
            }
        }
        if self.any_running() {
            context.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }

    fn accept_cue(&mut self, id: u64) {
        let Some(cue) = self.cues.iter_mut().find(|cue| cue.id == id) else {
            return;
        };
        let diff = cue.worktree.as_ref().map_or_else(
            || {
                Err(crate::workspace::WorkspaceError::Git(
                    "cue worktree was not created".to_owned(),
                ))
            },
            |worktree| {
                worktree
                    .open()
                    .and_then(|workspace| workspace.working_diff())
            },
        );
        match diff.and_then(|diff| {
            cue.session
                .accept(&diff)
                .map(|_| ())
                .map_err(|error| crate::workspace::WorkspaceError::Git(error.to_string()))
        }) {
            Ok(()) => cue.error = None,
            Err(error) => cue.error = Some(error.to_string()),
        }
    }

    fn commit_and_merge(&mut self, id: u64) {
        let Some(index) = self.cues.iter().position(|cue| cue.id == id) else {
            return;
        };
        if self.cues[index].branch_commit.is_none() {
            let approval = self.cues[index].session.approval().cloned();
            let Some(approval) = approval else {
                self.cues[index].error =
                    Some("accept this cue's diff before committing".to_owned());
                return;
            };
            let message = self.cues[index].commit_message.clone();
            let commit = self.cues[index].worktree.as_ref().map_or_else(
                || {
                    Err(crate::workspace::WorkspaceError::Git(
                        "cue worktree was not created".to_owned(),
                    ))
                },
                |cue_worktree| {
                    cue_worktree
                        .open()
                        .and_then(|mut worktree| worktree.commit_approved(&approval, &message))
                },
            );
            match commit {
                Ok(commit) => self.cues[index].branch_commit = Some(commit),
                Err(error) => {
                    self.cues[index].error = Some(error.to_string());
                    return;
                }
            }
        }
        let Some(main) = &mut self.workspace else {
            return;
        };
        let Some(worktree) = &self.cues[index].worktree else {
            self.cues[index].error = Some("cue worktree was not created".to_owned());
            return;
        };
        match main.merge_cue(worktree) {
            Ok(commit) => {
                let cue = &mut self.cues[index];
                if let Err(error) = cue.session.mark_committed(commit) {
                    cue.error = Some(error.to_string());
                    return;
                }
                cue.lane = CueLane::Done;
                cue.error = None;
                self.refresh();
            }
            Err(error) => self.cues[index].error = Some(error.to_string()),
        }
    }

    fn archive_cue(&mut self, id: u64) {
        let Some(index) = self.cues.iter().position(|cue| cue.id == id) else {
            return;
        };
        let Some(worktree) = self.cues[index].worktree.as_ref() else {
            self.cues[index].lane = CueLane::Archive;
            self.cues[index].error = None;
            if self.selected_cue == Some(id) {
                self.selected_cue = None;
            }
            return;
        };
        let Some(main) = &mut self.workspace else {
            return;
        };
        match main.archive_cue_worktree(worktree) {
            Ok(()) => {
                self.cues[index].lane = CueLane::Archive;
                self.cues[index].error = None;
                if self.selected_cue == Some(id) {
                    self.selected_cue = None;
                }
            }
            Err(error) => self.cues[index].error = Some(error.to_string()),
        }
    }

    fn reject_cue(&mut self, id: u64) {
        let Some(index) = self.cues.iter().position(|cue| cue.id == id) else {
            return;
        };
        let Some(main) = &mut self.workspace else {
            return;
        };
        let Some(worktree) = &self.cues[index].worktree else {
            self.cues[index].error = Some("cue worktree was not created".to_owned());
            return;
        };
        match main.archive_cue_worktree(worktree) {
            Ok(()) => {
                let cue = &mut self.cues[index];
                let _ = cue.session.reject();
                cue.lane = CueLane::Archive;
                cue.error = None;
                self.selected_cue = None;
            }
            Err(error) => self.cues[index].error = Some(error.to_string()),
        }
    }

    fn reject_confirmation_ui(&mut self, context: &egui::Context) {
        let Some(id) = self.confirm_reject else {
            return;
        };
        egui::Window::new("Reject isolated cue?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(context, |ui| {
                ui.label("This removes the cue worktree and permanently discards its branch.");
                ui.label("The main worktree will not be changed.");
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        self.confirm_reject = None;
                    }
                    if ui.button("Reject Cue").clicked() {
                        self.reject_cue(id);
                        self.confirm_reject = None;
                    }
                });
            });
    }

    fn save_settings(&mut self) {
        let Some(path) = &self.settings_path else {
            return;
        };
        if let Err(error) = settings::save(path, &self.settings) {
            self.error = Some(error.to_string());
        }
    }

    fn settings_ui(&mut self, context: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let mut open = self.settings_open;
        let mut should_save = false;
        egui::Window::new("Codex Settings")
            .open(&mut open)
            .resizable(true)
            .default_width(520.0)
            .show(context, |ui| {
                ui.label("Codex CLI path");
                ui.text_edit_singleline(&mut self.settings.codex_cli_path);
                ui.label("Model (blank uses Codex configuration)");
                ui.text_edit_singleline(&mut self.settings.codex_model);
                ui.label("Extra arguments");
                ui.text_edit_singleline(&mut self.settings.codex_extra_arguments);
                ui.label("Environment variable names (one per line; values are never saved)");
                ui.add(
                    egui::TextEdit::multiline(&mut self.settings.codex_environment_names)
                        .desired_rows(3),
                );
                ui.label("Pre-run command (executable and arguments; no shell)");
                ui.text_edit_singleline(&mut self.settings.codex_pre_run_command);
                ui.label("Post-run command (executable and arguments; no shell)");
                ui.text_edit_singleline(&mut self.settings.codex_post_run_command);
                if ui.button("Save Settings").clicked() {
                    should_save = true;
                }
            });
        self.settings_open = open;
        if should_save {
            self.save_settings();
        }
    }

    fn shortcuts(&mut self, context: &egui::Context) {
        if context.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) {
            self.choose_repository();
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::R)) {
            self.refresh();
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::Comma))
        {
            self.settings_open = true;
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.settings_open = false;
            self.confirm_reject = None;
        }
    }
}

fn render_card(ui: &mut egui::Ui, cue: &mut CueCard) -> Option<CardAction> {
    let mut action = None;
    ui.group(|ui| {
        ui.set_min_width(220.0);
        ui.strong(format!(
            "#{}  {}",
            cue.id,
            truncate(cue.session.cue().instruction(), 42)
        ));
        if let Some(worktree) = &cue.worktree {
            ui.small(worktree.branch());
        } else {
            ui.small("Queued; worktree not created");
        }
        if let Some(error) = &cue.error {
            ui.colored_label(ui.visuals().error_fg_color, truncate(error, 90));
        }
        match cue.lane {
            CueLane::Inbox => {
                if ui.button("Run Cue").clicked() {
                    action = Some(CardAction::Run(cue.id));
                }
                if ui.button("Archive").clicked() {
                    action = Some(CardAction::Archive(cue.id));
                }
            }
            CueLane::Run => match cue.session.state() {
                SessionState::Ready => {
                    if ui.button("Run in Worktree").clicked() {
                        action = Some(CardAction::Run(cue.id));
                    }
                }
                SessionState::Running { .. } => {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().color(theme::CODEX_ACCENT));
                        ui.label("Codex running");
                    });
                    if ui.button("Cancel").clicked() {
                        action = Some(CardAction::Cancel(cue.id));
                    }
                }
                _ => {}
            },
            CueLane::Review => {
                if ui.button("Open Review").clicked() {
                    action = Some(CardAction::Select(cue.id));
                }
                if cue.branch_commit.is_some() {
                    ui.small("Committed; merge can be retried safely.");
                }
            }
            CueLane::Done => {
                ui.label("Merged into main");
                if ui.button("Archive").clicked() {
                    action = Some(CardAction::Archive(cue.id));
                }
            }
            CueLane::Archive => {
                ui.label("Archived");
            }
        }
    });
    action
}

fn truncate(text: &str, limit: usize) -> String {
    let mut characters = text.chars();
    let truncated: String = characters.by_ref().take(limit).collect();
    if characters.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

impl eframe::App for CodexDirigentApp {
    fn logic(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_codex(context);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.shortcuts(ui.ctx());
        self.settings_ui(ui.ctx());
        self.reject_confirmation_ui(ui.ctx());
        egui::Panel::top("toolbar").show_inside(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("status_bar").show_inside(ui, |ui| self.status_bar(ui));

        if self.workspace.is_some() {
            self.workspace_ui(ui);
        } else {
            egui::CentralPanel::default().show_inside(ui, |ui| {
                ui.vertical_centered_justified(|ui| {
                    ui.add_space(120.0);
                    ui.heading("Open a local Git repository");
                    ui.label("Run unlimited isolated cues and merge reviewed work into main.");
                    ui.add_space(16.0);
                    if ui
                        .add(
                            egui::Button::new("Open Repository…")
                                .fill(theme::CODEX_ACCENT)
                                .min_size(egui::vec2(180.0, 36.0)),
                        )
                        .clicked()
                    {
                        self.choose_repository();
                    }
                });
            });
        }
    }
}

fn with_line_numbers(text: &str) -> String {
    let line_count = text.lines().count().max(1);
    let width = line_count.ilog10() as usize + 1;
    let mut numbered = String::with_capacity(text.len() + line_count * (width + 3));
    for (index, line) in text.lines().enumerate() {
        let _ = writeln!(numbered, "{:>width$} │ {line}", index + 1);
    }
    if text.is_empty() {
        numbered.push_str("1 │ ");
    }
    numbered
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    fn git(repository: &std::path::Path, arguments: &[&str]) {
        let output = Command::new("git")
            .args(arguments)
            .current_dir(repository)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn repository() -> tempfile::TempDir {
        let repository = tempfile::tempdir().unwrap();
        git(repository.path(), &["init", "-b", "main"]);
        git(repository.path(), &["config", "user.name", "Test User"]);
        git(
            repository.path(),
            &["config", "user.email", "test@example.com"],
        );
        fs::write(repository.path().join("README.md"), "test\n").unwrap();
        git(repository.path(), &["add", "README.md"]);
        git(repository.path(), &["commit", "-m", "Initial commit"]);
        repository
    }

    #[test]
    fn starts_empty_in_browse_view() {
        let app = CodexDirigentApp::default();
        assert_eq!(app.view, AppView::Browse);
        assert!(app.cues.is_empty());
    }

    #[test]
    fn cue_lanes_match_requested_order() {
        let labels: Vec<_> = CueLane::ALL.into_iter().map(CueLane::label).collect();
        assert_eq!(labels, ["Inbox", "Run", "Review", "Done", "Archive"]);
    }

    #[test]
    fn new_cues_wait_in_inbox_without_a_worktree() {
        let cue = Cue::new("Update the docs", CueTarget::Repository).unwrap();
        let card = CueCard::new(1, cue);
        assert_eq!(card.lane, CueLane::Inbox);
        assert!(card.worktree.is_none());
        assert_eq!(card.session.state(), &SessionState::Ready);
    }

    #[test]
    fn run_inbox_prepares_every_cue_and_moves_it_to_run() {
        let repository = repository();
        let mut app = CodexDirigentApp {
            workspace: Some(Workspace::open(repository.path()).unwrap()),
            ..CodexDirigentApp::default()
        };
        for id in 1..=3 {
            let cue = Cue::new(format!("Cue {id}"), CueTarget::Repository).unwrap();
            app.cues.push(CueCard::new(id, cue));
        }
        // Make process setup fail synchronously after worktree creation.
        app.settings.codex_extra_arguments = "\"unterminated".to_owned();

        app.run_inbox();

        assert!(app.cues.iter().all(|cue| cue.lane == CueLane::Run));
        assert!(app.cues.iter().all(|cue| cue.worktree.is_some()));
        assert!(app.cues.iter().all(|cue| !cue.is_running()));

        for id in 1..=3 {
            app.archive_cue(id);
        }
        assert!(app.cues.iter().all(|cue| cue.lane == CueLane::Archive));
    }

    #[test]
    fn viewer_adds_aligned_line_numbers() {
        let mut source = String::new();
        for line in 1..=12 {
            let _ = writeln!(source, "line {line}");
        }
        let numbered = with_line_numbers(&source);
        assert!(numbered.starts_with(" 1 │ line 1"));
        assert!(numbered.contains("12 │ line 12"));
    }

    #[test]
    fn card_text_is_unicode_safe() {
        assert_eq!(truncate("conduct 🎼 carefully", 9), "conduct 🎼…");
    }
}
