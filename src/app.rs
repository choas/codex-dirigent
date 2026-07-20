use eframe::egui;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::{
    PRODUCT_NAME,
    board::{self, BoardState, PersistedCue, PersistedLane},
    codex::{self, CodexEvent, CodexRun},
    cue::{Cue, CueTarget},
    review::{Session, SessionState},
    settings::{self, Settings},
    theme,
    workspace::{CueWorktree, FileEntry, Workspace},
};

#[derive(Debug, Default)]
struct FileTree {
    directories: BTreeMap<OsString, Self>,
    files: Vec<FileEntry>,
}

impl FileTree {
    fn from_files(files: &[FileEntry]) -> Self {
        let mut tree = Self::default();
        for file in files {
            tree.insert(file.clone());
        }
        tree
    }

    fn insert(&mut self, file: FileEntry) {
        let mut directory = self;
        if let Some(parent) = file.relative_path.parent() {
            for component in parent.components() {
                directory = directory
                    .directories
                    .entry(component.as_os_str().to_os_string())
                    .or_default();
            }
        }
        directory.files.push(file);
    }

    fn show(
        &self,
        ui: &mut egui::Ui,
        directory_path: &Path,
        selected_file: Option<&PathBuf>,
        clicked_file: &mut Option<PathBuf>,
    ) {
        for (name, contents) in &self.directories {
            let path = directory_path.join(name);
            egui::CollapsingHeader::new(name.to_string_lossy())
                .id_salt(&path)
                .show(ui, |ui| {
                    contents.show(ui, &path, selected_file, clicked_file);
                });
        }

        for file in &self.files {
            let marker = file
                .status
                .map_or("  ".to_owned(), |status| format!("{status} "));
            let name = file
                .relative_path
                .file_name()
                .unwrap_or(file.relative_path.as_os_str())
                .to_string_lossy();
            let label = format!("{marker}{name}");
            let selected = selected_file == Some(&file.relative_path);
            if ui
                .selectable_label(selected, label)
                .on_hover_text(file.relative_path.display().to_string())
                .clicked()
            {
                *clicked_file = Some(file.relative_path.clone());
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppView {
    Browse,
    Board,
    CueDetail,
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
    board_path: Option<PathBuf>,
    loaded_board: Option<BoardState>,
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
            board_path: None,
            loaded_board: None,
            settings_open: false,
        }
    }
}

impl CodexDirigentApp {
    #[must_use]
    pub fn load() -> Self {
        let path = settings::default_path().ok();
        let (loaded_settings, settings_warning) = path.as_ref().map_or_else(
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
        let board_path = path
            .as_ref()
            .map(|settings_path| settings_path.with_file_name(board::FILE_NAME));
        let (loaded_board, board_warning) = board_path.as_ref().map_or_else(
            || (BoardState::default(), None),
            |board_path| board::load_or_empty(board_path),
        );
        let recent = loaded_settings.last_repository.clone();
        let mut app = Self {
            settings: loaded_settings,
            settings_path: path,
            board_path,
            loaded_board: Some(loaded_board),
            ..Self::default()
        };
        if let Some(repository) = recent
            && repository.exists()
        {
            app.open_repository(&repository);
        }
        for warning in [settings_warning, board_warning].into_iter().flatten() {
            app.append_warning(warning);
        }
        app
    }

    fn append_warning(&mut self, warning: impl Into<String>) {
        let warning = warning.into();
        self.error = Some(
            self.error
                .take()
                .map_or(warning.clone(), |existing| format!("{existing}\n{warning}")),
        );
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
                let persisted_board = self.loaded_board.take().unwrap_or_default();
                self.settings.last_repository = Some(workspace.root().to_path_buf());
                self.workspace = Some(workspace);
                self.selected_file = None;
                self.file_text.clear();
                self.cues.clear();
                self.selected_cue = None;
                self.error = None;
                self.view = AppView::Browse;
                self.save_settings();
                self.recover_linked_cues(&persisted_board);
                self.save_board();
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

    fn recover_linked_cues(&mut self, persisted_board: &BoardState) {
        let linked = match self.workspace.as_ref().map(Workspace::linked_cue_worktrees) {
            Some(Ok(linked)) => linked,
            Some(Err(error)) => {
                self.error = Some(format!("could not recover unfinished cues: {error}"));
                return;
            }
            None => return,
        };
        let repository = self
            .workspace
            .as_ref()
            .map(|workspace| workspace.root().to_path_buf());
        let metadata_matches = persisted_board.repository == repository;
        let mut linked_by_branch: BTreeMap<_, _> = linked
            .into_iter()
            .map(|worktree| (worktree.branch().to_owned(), worktree))
            .collect();
        let (mut recovered_review_ids, stale_count) = if metadata_matches {
            self.recover_persisted_cues(persisted_board, &mut linked_by_branch)
        } else if !persisted_board.cues.is_empty() {
            self.append_warning(
                "saved cue-board state belongs to a different repository and was ignored",
            );
            (Vec::new(), 0)
        } else {
            (Vec::new(), 0)
        };
        let fallback_ids = self.recover_worktrees_without_metadata(linked_by_branch);
        let fallback_count = fallback_ids.len();
        recovered_review_ids.extend(fallback_ids);
        if let Some(id) = recovered_review_ids.last().copied() {
            self.selected_cue = Some(id);
            self.view = AppView::CueDetail;
        } else if !self.cues.is_empty() {
            self.view = AppView::Board;
        }
        if fallback_count > 0 {
            self.append_warning(format!(
                "Recovered {fallback_count} unfinished cue worktree(s) without persisted metadata. Review each diff carefully."
            ));
        }
        if stale_count > 0 {
            self.append_warning(format!(
                "Ignored {stale_count} stale persisted cue entr{} with no matching linked worktree.",
                if stale_count == 1 { "y" } else { "ies" }
            ));
        }
    }

    fn recover_persisted_cues(
        &mut self,
        persisted_board: &BoardState,
        linked_by_branch: &mut BTreeMap<String, CueWorktree>,
    ) -> (Vec<u64>, usize) {
        self.next_cue_id = self.next_cue_id.max(persisted_board.next_cue_id);
        let mut used_ids = BTreeSet::new();
        let mut review_ids = Vec::new();
        let mut stale_count = 0_usize;
        for persisted in &persisted_board.cues {
            self.next_cue_id = self.next_cue_id.max(persisted.id.saturating_add(1));
            let Ok(cue) = persisted.cue() else {
                stale_count += 1;
                continue;
            };
            if !used_ids.insert(persisted.id) {
                stale_count += 1;
                continue;
            }
            let Some(branch) = persisted.worktree_branch.as_deref() else {
                if persisted.lane == PersistedLane::Inbox {
                    let mut card = CueCard::new(persisted.id, cue.clone());
                    card.session = Session::recover_ready(cue, persisted.follow_ups.clone());
                    card.commit_message.clone_from(&persisted.commit_message);
                    self.cues.push(card);
                } else {
                    stale_count += 1;
                }
                continue;
            };
            let Some(worktree) = linked_by_branch.remove(branch) else {
                stale_count += 1;
                continue;
            };
            if let Some(card) = self.recover_persisted_worktree(persisted, cue, worktree) {
                if card.lane == CueLane::Review {
                    review_ids.push(card.id);
                }
                self.cues.push(card);
            }
        }
        (review_ids, stale_count)
    }

    fn recover_persisted_worktree(
        &mut self,
        persisted: &PersistedCue,
        cue: Cue,
        worktree: CueWorktree,
    ) -> Option<CueCard> {
        let diff = match worktree
            .open()
            .and_then(|workspace| workspace.working_diff())
        {
            Ok(diff) => diff,
            Err(error) => {
                self.append_warning(format!(
                    "could not recover cue branch `{}`: {error}",
                    worktree.branch()
                ));
                return None;
            }
        };
        let branch_commit = persisted
            .branch_commit
            .as_ref()
            .filter(|commit| branch_commit_is_live(&worktree, commit))
            .cloned();
        if persisted.branch_commit.is_some() && branch_commit.is_none() {
            self.append_warning(format!(
                "discarded stale committed-branch metadata for `{}`; review is required again",
                worktree.branch()
            ));
        }
        let merged_commit = (persisted.lane == PersistedLane::Done && branch_commit.is_some())
            .then_some(persisted.merged_commit.as_ref())
            .flatten()
            .filter(|commit| {
                self.workspace
                    .as_ref()
                    .is_some_and(|main| main.contains_commit(commit).unwrap_or(false))
            })
            .cloned();
        if persisted.lane == PersistedLane::Done && merged_commit.is_none() {
            self.append_warning(format!(
                "discarded stale merged-cue metadata for `{}`; the live branch remains recoverable",
                worktree.branch()
            ));
        }
        let (session, lane) = if let Some(commit) = merged_commit {
            (
                Session::recover_done(cue, persisted.follow_ups.clone(), commit),
                CueLane::Done,
            )
        } else if branch_commit.is_some() {
            (
                Session::recover_committed_branch(cue, persisted.follow_ups.clone(), diff),
                CueLane::Review,
            )
        } else {
            (
                Session::recover_reviewing(cue, persisted.follow_ups.clone(), diff),
                CueLane::Review,
            )
        };
        let mut card = CueCard::new(persisted.id, session.cue().clone());
        card.session = session;
        card.lane = lane;
        card.worktree = Some(worktree);
        card.branch_commit = branch_commit;
        card.commit_message.clone_from(&persisted.commit_message);
        Some(card)
    }

    fn recover_worktrees_without_metadata(
        &mut self,
        linked_by_branch: BTreeMap<String, CueWorktree>,
    ) -> Vec<u64> {
        let mut recovered = Vec::new();
        for worktree in linked_by_branch.into_values() {
            let diff = match worktree
                .open()
                .and_then(|workspace| workspace.working_diff())
            {
                Ok(diff) => diff,
                Err(error) => {
                    self.append_warning(format!(
                        "could not recover cue branch `{}`: {error}",
                        worktree.branch()
                    ));
                    continue;
                }
            };
            let instruction = format!("Recovered unfinished cue from `{}`", worktree.branch());
            let Ok(cue) = Cue::new(instruction, CueTarget::Repository) else {
                continue;
            };
            let id = self.next_cue_id;
            self.next_cue_id += 1;
            let mut card = CueCard::new(id, cue);
            let Ok(run_id) = card.session.begin_run() else {
                continue;
            };
            if card
                .session
                .finish_run(run_id, "Recovered after application restart", diff)
                .is_err()
            {
                continue;
            }
            card.lane = CueLane::Review;
            card.worktree = Some(worktree);
            recovered.push(id);
            self.cues.push(card);
        }
        recovered
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
        ui.horizontal_wrapped(|ui| {
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
        let file_tree = self
            .workspace
            .as_ref()
            .map(|workspace| FileTree::from_files(workspace.files()))
            .unwrap_or_default();
        let mut clicked_file = None;
        egui::Panel::left("file_tree")
            .resizable(true)
            .default_size(260.0)
            .show_inside(ui, |ui| {
                ui.heading("Files");
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    file_tree.show(
                        ui,
                        Path::new(""),
                        self.selected_file.as_ref(),
                        &mut clicked_file,
                    );
                });
            });
        if let Some(path) = clicked_file {
            self.select_file(path);
        }

        egui::CentralPanel::default().show_inside(ui, |ui| match self.view {
            AppView::Browse => self.browser_ui(ui),
            AppView::Board => self.board_ui(ui),
            AppView::CueDetail => self.cue_detail_ui(ui),
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
        self.save_board();
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
        egui::ScrollArea::vertical()
            .id_salt("cue_board")
            .show(ui, |ui| {
                for lane in CueLane::ALL {
                    let count = self.cues.iter().filter(|cue| cue.lane == lane).count();
                    egui::CollapsingHeader::new(format!("{}  {count}", lane.label()))
                        .id_salt(("cue_lane", lane.label()))
                        .default_open(count > 0)
                        .show(ui, |ui| {
                            if count == 0 {
                                ui.weak("No cues");
                                return;
                            }

                            let spacing = ui.spacing().item_spacing.x;
                            let card_width = cue_card_width(ui.available_width(), spacing);
                            ui.horizontal_wrapped(|ui| {
                                for cue in cues_in_lane_newest_first(&mut self.cues, lane) {
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(card_width, 0.0),
                                        egui::Layout::top_down(egui::Align::Min),
                                        |ui| {
                                            if let Some(next) = render_card(ui, cue) {
                                                action = Some(next);
                                            }
                                        },
                                    );
                                }
                            });
                        });
                    if lane != CueLane::Archive {
                        ui.separator();
                    }
                }
            });
        if let Some(action) = action {
            self.handle_card_action(action);
        }
    }

    fn handle_card_action(&mut self, action: CardAction) {
        match action {
            CardAction::Select(id) => {
                self.selected_cue = Some(id);
                self.view = AppView::CueDetail;
            }
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

    fn cue_detail_ui(&mut self, ui: &mut egui::Ui) {
        if ui.button("← Cue Board").clicked() {
            self.view = AppView::Board;
            return;
        }
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("cue_detail")
            .show(ui, |ui| self.selected_cue_ui(ui));
    }

    fn selected_cue_ui(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.selected_cue else {
            self.view = AppView::Board;
            return;
        };
        let Some(index) = self.cues.iter().position(|cue| cue.id == id) else {
            self.selected_cue = None;
            self.view = AppView::Board;
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
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        let mut diff = cue.session.review_diff().to_owned();
                        let available_width = ui.available_width();
                        ui.add(
                            egui::TextEdit::multiline(&mut diff)
                                .font(egui::TextStyle::Monospace)
                                .interactive(false)
                                .desired_width(available_width),
                        );
                    });
                let reviewing = matches!(cue.session.state(), SessionState::Reviewing { .. });
                let accepted = cue.session.state() == &SessionState::Accepted;
                if reviewing {
                    ui.horizontal_wrapped(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut cue.follow_up).desired_width(180.0));
                        send_follow_up = ui.button("Send Follow-up").clicked();
                    });
                }
                ui.horizontal_wrapped(|ui| {
                    accept = ui
                        .add_enabled(reviewing, egui::Button::new("Accept Diff"))
                        .clicked();
                    reject = ui
                        .add_enabled(reviewing || accepted, egui::Button::new("Reject Cue"))
                        .clicked();
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label("Commit message");
                    ui.add(
                        egui::TextEdit::singleline(&mut cue.commit_message).desired_width(180.0),
                    );
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
        self.save_board();
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
                self.save_board();
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
        let mut board_changed = false;
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
                    board_changed = true;
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
                    board_changed = true;
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
        if board_changed {
            self.save_board();
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
            Ok(()) => {
                cue.error = None;
                self.save_board();
            }
            Err(error) => cue.error = Some(error.to_string()),
        }
    }

    fn commit_and_merge(&mut self, id: u64) {
        let Some(index) = self.cues.iter().position(|cue| cue.id == id) else {
            return;
        };
        let recovered_committed_branch = self.cues[index].branch_commit.is_some()
            && self.cues[index].session.approval().is_none();
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
                Ok(commit) => {
                    self.cues[index].branch_commit = Some(commit);
                    self.save_board();
                }
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
                let transition = if recovered_committed_branch {
                    cue.session.mark_recovered_branch_merged(commit)
                } else {
                    cue.session.mark_committed(commit)
                };
                if let Err(error) = transition {
                    cue.error = Some(error.to_string());
                    return;
                }
                cue.lane = CueLane::Done;
                cue.error = None;
                self.refresh();
                self.save_board();
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
                self.view = AppView::Board;
            }
            self.save_board();
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
                    self.view = AppView::Board;
                }
                self.save_board();
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
                self.view = AppView::Board;
                self.save_board();
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

    fn persisted_board(&self) -> Option<BoardState> {
        let repository = self.workspace.as_ref()?.root().to_path_buf();
        let mut state = BoardState::for_repository(repository, self.next_cue_id);
        state.cues = self
            .cues
            .iter()
            .filter(|cue| {
                cue.lane != CueLane::Archive && cue.session.state() != &SessionState::Rejected
            })
            .map(|card| {
                let lane = match card.lane {
                    CueLane::Inbox => PersistedLane::Inbox,
                    CueLane::Run => PersistedLane::Run,
                    CueLane::Review => PersistedLane::Review,
                    CueLane::Done => PersistedLane::Done,
                    CueLane::Archive => unreachable!("archived cues were filtered"),
                };
                let mut cue = PersistedCue::new(card.id, lane, card.session.cue());
                cue.follow_ups = card.session.user_follow_ups().map(str::to_owned).collect();
                cue.worktree_branch = card
                    .worktree
                    .as_ref()
                    .map(|worktree| worktree.branch().to_owned());
                cue.branch_commit.clone_from(&card.branch_commit);
                cue.merged_commit = match card.session.state() {
                    SessionState::Committed { commit } => Some(commit.clone()),
                    _ => None,
                };
                cue.commit_message.clone_from(&card.commit_message);
                cue
            })
            .collect();
        Some(state)
    }

    fn save_board(&mut self) {
        let (Some(path), Some(state)) = (self.board_path.clone(), self.persisted_board()) else {
            return;
        };
        if let Err(error) = board::save(&path, &state) {
            self.append_warning(error.to_string());
        }
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
                ui.label("Model (defaults to GPT-5.6 for Codex)");
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
        ui.set_width(ui.available_width());
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
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Run Cue").clicked() {
                        action = Some(CardAction::Run(cue.id));
                    }
                    if ui.button("Archive").clicked() {
                        action = Some(CardAction::Archive(cue.id));
                    }
                });
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

fn branch_commit_is_live(worktree: &CueWorktree, expected_commit: &str) -> bool {
    worktree.open().is_ok_and(|workspace| {
        workspace.is_clean().unwrap_or(false)
            && workspace
                .head_commit()
                .is_ok_and(|commit| commit == expected_commit)
    })
}

fn cues_in_lane_newest_first(
    cues: &mut [CueCard],
    lane: CueLane,
) -> impl Iterator<Item = &mut CueCard> {
    cues.iter_mut().rev().filter(move |cue| cue.lane == lane)
}

fn cue_card_width(available_width: f32, spacing: f32) -> f32 {
    const MIN_CARD_WIDTH: f32 = 190.0;
    const IDEAL_CARD_WIDTH: f32 = 240.0;

    let columns = ((available_width + spacing) / (MIN_CARD_WIDTH + spacing))
        .floor()
        .max(1.0);
    ((available_width - spacing * (columns - 1.0)) / columns).clamp(0.0, IDEAL_CARD_WIDTH)
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
    fn opening_review_navigates_to_a_dedicated_detail_view() {
        let mut app = CodexDirigentApp::default();
        app.handle_card_action(CardAction::Select(42));
        assert_eq!(app.selected_cue, Some(42));
        assert_eq!(app.view, AppView::CueDetail);
    }

    #[test]
    fn cues_in_each_lane_are_listed_newest_first() {
        let mut cues = vec![
            CueCard::new(1, Cue::new("Old inbox cue", CueTarget::Repository).unwrap()),
            CueCard::new(2, Cue::new("Review cue", CueTarget::Repository).unwrap()),
            CueCard::new(
                3,
                Cue::new("Latest inbox cue", CueTarget::Repository).unwrap(),
            ),
        ];
        cues[1].lane = CueLane::Review;

        let inbox_ids: Vec<_> = cues_in_lane_newest_first(&mut cues, CueLane::Inbox)
            .map(|cue| cue.id)
            .collect();
        assert_eq!(inbox_ids, [3, 1]);
    }

    #[test]
    fn cue_cards_fit_the_available_board_width() {
        assert!((cue_card_width(160.0, 8.0) - 160.0).abs() < f32::EPSILON);
        assert!(cue_card_width(420.0, 8.0) <= 206.0);
        assert!(cue_card_width(900.0, 8.0) <= 240.0);
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
    fn file_tree_groups_files_by_directory() {
        let files = vec![
            FileEntry {
                relative_path: PathBuf::from("README.md"),
                status: None,
            },
            FileEntry {
                relative_path: PathBuf::from("src/app.rs"),
                status: Some('M'),
            },
            FileEntry {
                relative_path: PathBuf::from("src/nested/mod.rs"),
                status: None,
            },
        ];

        let tree = FileTree::from_files(&files);
        assert_eq!(tree.files[0].relative_path, Path::new("README.md"));
        let src = &tree.directories[std::ffi::OsStr::new("src")];
        assert_eq!(src.files[0].status, Some('M'));
        assert_eq!(
            src.directories[std::ffi::OsStr::new("nested")].files[0].relative_path,
            Path::new("src/nested/mod.rs")
        );
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
    fn opening_repository_recovers_unfinished_cue_worktrees_for_review() {
        let repository = repository();
        let main = Workspace::open(repository.path()).unwrap();
        let worktree = main.create_cue_worktree(77).unwrap();
        fs::write(worktree.path().join("README.md"), "recovered change\n").unwrap();

        let mut app = CodexDirigentApp::default();
        app.open_repository(repository.path());

        assert_eq!(app.cues.len(), 1);
        assert_eq!(app.cues[0].lane, CueLane::Review);
        assert!(
            app.cues[0]
                .session
                .review_diff()
                .contains("recovered change")
        );
        assert_eq!(app.selected_cue, Some(app.cues[0].id));
        assert_eq!(app.view, AppView::CueDetail);

        app.reject_cue(app.cues[0].id);
        assert_eq!(app.cues[0].lane, CueLane::Archive);
    }

    #[test]
    fn persisted_inbox_recovers_instruction_and_exact_target() {
        let repository = repository();
        let state_directory = tempfile::tempdir().unwrap();
        let board_path = state_directory.path().join(board::FILE_NAME);
        let cue = Cue::new(
            "Explain this boundary",
            CueTarget::Lines {
                path: PathBuf::from("README.md"),
                start: 1,
                end: 1,
            },
        )
        .unwrap();
        let mut first = CodexDirigentApp {
            workspace: Some(Workspace::open(repository.path()).unwrap()),
            board_path: Some(board_path.clone()),
            next_cue_id: 2,
            ..CodexDirigentApp::default()
        };
        first.cues.push(CueCard::new(1, cue.clone()));
        first.save_board();

        let mut restarted = CodexDirigentApp {
            board_path: Some(board_path.clone()),
            loaded_board: Some(board::load(&board_path).unwrap()),
            ..CodexDirigentApp::default()
        };
        restarted.open_repository(repository.path());

        assert_eq!(restarted.cues.len(), 1);
        assert_eq!(restarted.cues[0].lane, CueLane::Inbox);
        assert_eq!(restarted.cues[0].session.cue(), &cue);
        assert!(restarted.cues[0].worktree.is_none());
        assert_eq!(restarted.next_cue_id, 2);
    }

    #[test]
    fn persisted_conversation_recovers_user_history_without_generated_output() {
        let repository = repository();
        let main = Workspace::open(repository.path()).unwrap();
        let worktree = main.create_cue_worktree(9).unwrap();
        fs::write(worktree.path().join("README.md"), "durable change\n").unwrap();
        let diff = worktree.open().unwrap().working_diff().unwrap();
        let cue = Cue::new(
            "Change the heading",
            CueTarget::File(PathBuf::from("README.md")),
        )
        .unwrap();
        let mut session = Session::new(cue.clone());
        let first_run = session.begin_run().unwrap();
        session
            .finish_run(first_run, "generated first summary", &diff)
            .unwrap();
        let second_run = session.follow_up("Keep the example concise").unwrap();
        session
            .finish_run(second_run, "generated second summary", &diff)
            .unwrap();
        let mut card = CueCard::new(9, cue.clone());
        card.session = session;
        card.lane = CueLane::Review;
        card.worktree = Some(worktree);
        let state_directory = tempfile::tempdir().unwrap();
        let board_path = state_directory.path().join(board::FILE_NAME);
        let mut first = CodexDirigentApp {
            workspace: Some(main),
            board_path: Some(board_path.clone()),
            next_cue_id: 10,
            ..CodexDirigentApp::default()
        };
        first.cues.push(card);
        first.save_board();

        let serialized = fs::read_to_string(&board_path).unwrap();
        assert!(!serialized.contains("generated first summary"));
        assert!(!serialized.contains("generated second summary"));
        assert!(!serialized.contains("durable change"));
        let mut restarted = CodexDirigentApp {
            board_path: Some(board_path.clone()),
            loaded_board: Some(board::load(&board_path).unwrap()),
            ..CodexDirigentApp::default()
        };
        restarted.open_repository(repository.path());

        let recovered = &restarted.cues[0];
        assert_eq!(recovered.session.cue(), &cue);
        assert_eq!(
            recovered
                .session
                .messages()
                .iter()
                .map(|message| message.text.as_str())
                .collect::<Vec<_>>(),
            ["Change the heading", "Keep the example concise"]
        );
        assert!(recovered.session.review_diff().contains("durable change"));
        assert!(matches!(
            recovered.session.state(),
            SessionState::Reviewing { .. }
        ));

        restarted.reject_cue(9);
    }

    #[test]
    fn stale_persisted_active_cue_cannot_fabricate_a_worktree() {
        let repository = repository();
        let root = Workspace::open(repository.path())
            .unwrap()
            .root()
            .to_path_buf();
        let cue = Cue::new("Stale task", CueTarget::Repository).unwrap();
        let mut persisted = PersistedCue::new(5, PersistedLane::Review, &cue);
        persisted.worktree_branch = Some("codex-dirigent/cue-5-missing".to_owned());
        let mut state = BoardState::for_repository(root, 6);
        state.cues.push(persisted);
        let state_directory = tempfile::tempdir().unwrap();
        let board_path = state_directory.path().join(board::FILE_NAME);
        board::save(&board_path, &state).unwrap();
        let mut restarted = CodexDirigentApp {
            board_path: Some(board_path.clone()),
            loaded_board: Some(board::load(&board_path).unwrap()),
            ..CodexDirigentApp::default()
        };

        restarted.open_repository(repository.path());

        assert!(restarted.cues.is_empty());
        assert!(
            restarted
                .error
                .as_deref()
                .unwrap()
                .contains("stale persisted")
        );
    }

    #[test]
    fn archived_and_rejected_cards_are_removed_from_active_persistence() {
        let repository = repository();
        let mut archived = CueCard::new(1, Cue::new("Archived", CueTarget::Repository).unwrap());
        archived.lane = CueLane::Archive;
        let mut rejected = CueCard::new(2, Cue::new("Rejected", CueTarget::Repository).unwrap());
        let run = rejected.session.begin_run().unwrap();
        rejected.session.finish_run(run, "done", "diff").unwrap();
        rejected.session.reject().unwrap();
        rejected.lane = CueLane::Review;
        let app = CodexDirigentApp {
            workspace: Some(Workspace::open(repository.path()).unwrap()),
            cues: vec![archived, rejected],
            ..CodexDirigentApp::default()
        };

        assert!(app.persisted_board().unwrap().cues.is_empty());
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
