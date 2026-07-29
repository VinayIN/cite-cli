use std::path::{Component, Path, PathBuf};

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Tabs, Wrap,
    },
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::core::CiteError;
use crate::core::db::DbManager;
use crate::core::project::{
    self, AllStats, ProjectContext, ProjectStats, StoredBuild, StoredDeployment, StoredTimeline,
};
use crate::core::{compiler, deploy, doctor, scaffold};

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

pub struct Cmd {
    pub label: &'static str,
    pub desc: &'static str,
    pub args_hint: &'static str,
    pub needs_project: bool,
    pub id: CommandId,
}

#[derive(Clone, Copy, PartialEq)]
pub enum CommandId {
    Init,
    Build,
    Lint,
    Status,
    Doctor,
    Deploy,
    Rollback,
    Clean,
}

pub const CMDS: &[Cmd] = &[
    Cmd {
        label: "init",
        desc: "Create a new project with starter files",
        args_hint: "<name>",
        needs_project: false,
        id: CommandId::Init,
    },
    Cmd {
        label: "build",
        desc: "Execute the compiler protocol and build artifact",
        args_hint: "[--force]",
        needs_project: true,
        id: CommandId::Build,
    },
    Cmd {
        label: "lint",
        desc: "Run linting rules (naming, style, word counts)",
        args_hint: "",
        needs_project: true,
        id: CommandId::Lint,
    },
    Cmd {
        label: "status",
        desc: "Show project health, validation, and sync state",
        args_hint: "",
        needs_project: true,
        id: CommandId::Status,
    },
    Cmd {
        label: "doctor",
        desc: "Diagnose common project issues and configuration",
        args_hint: "",
        needs_project: true,
        id: CommandId::Doctor,
    },
    Cmd {
        label: "deploy",
        desc: "Deploy to Supabase (--staging for local cite.db)",
        args_hint: "[--dry-run] [--staging]",
        needs_project: true,
        id: CommandId::Deploy,
    },
    Cmd {
        label: "rollback",
        desc: "Roll back to the previous deployment",
        args_hint: "<deployment id>",
        needs_project: true,
        id: CommandId::Rollback,
    },
    Cmd {
        label: "clean",
        desc: "Remove build artifacts, cache, and temp files",
        args_hint: "",
        needs_project: true,
        id: CommandId::Clean,
    },
];

#[derive(Clone, Copy, PartialEq)]
pub enum TuiMode {
    Runner,
    Analytics,
    ProjectView,
    CommandPalette,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Focus {
    Projects,
    Commands,
    Details,
    Logs,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ProjectTab {
    Overview,
    Podcasts,
    Timelines,
    Builds,
    Deployments,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ProjectListItemType {
    LocalHeader,
    ArchivedHeader,
    LocalProject(usize),
    ArchivedProject,
}

pub struct AnalyticsState {
    pub projects_state: ListState,
    pub stats: Option<ProjectStats>,
    pub global: Option<AllStats>,
}

pub struct ProjectViewState {
    pub projects_state: ListState,
    pub project_stats: Option<project::ProjectStats>,
    pub podcasts: Vec<project::StoredPodcast>,
    pub podcasts_state: ListState,
    pub timelines: Vec<StoredTimeline>,
    pub timelines_state: ListState,
    pub builds: Vec<StoredBuild>,
    pub builds_state: ListState,
    pub deploys: Vec<StoredDeployment>,
    pub deploys_state: ListState,
    pub sel_tab: ProjectTab,
}

pub struct CommandPaletteState {
    pub search: String,
    pub list_state: ListState,
}

struct EditorPick {
    files: Vec<PathBuf>,
    state: ListState,
}

pub struct AppState {
    cwd: PathBuf,
    pub roots: Vec<PathBuf>,
    pub db_projects: Vec<(String, String)>,
    pub projects_state: ListState,

    pub focus: Focus,
    pub cmds_state: ListState,

    pub log: Vec<String>,
    pub scroll: usize,
    pub busy: bool,
    pub arg_input: String,

    editor_pick: Option<EditorPick>,
    pending_edit: Option<PathBuf>,

    rx: mpsc::Receiver<()>,
    tx: mpsc::Sender<()>,
    task: Option<JoinHandle<()>>,

    mode: TuiMode,
    analytics: AnalyticsState,
    project_view: ProjectViewState,
    command_palette: CommandPaletteState,

    local_expanded: bool,
    archived_expanded: bool,
}

impl AppState {
    pub async fn new(cwd: &Path) -> Self {
        let (tx, rx) = mpsc::channel(32);
        let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
        let mut roots = project::discover_projects(&cwd);
        roots.sort();

        let db_projects = Self::load_db_projects().await;

        let mut projects_state = ListState::default();
        if !roots.is_empty() {
            projects_state.select(Some(0));
        }

        let mut cmds_state = ListState::default();
        cmds_state.select(Some(0));

        Self {
            cwd: cwd.to_path_buf(),
            roots,
            db_projects,
            projects_state,
            focus: Focus::Commands,
            cmds_state,
            log: vec![],
            scroll: 0,
            busy: false,
            arg_input: String::new(),
            editor_pick: None,
            pending_edit: None,
            rx,
            tx,
            task: None,
            mode: TuiMode::Runner,
            analytics: AnalyticsState {
                projects_state: ListState::default(),
                stats: None,
                global: None,
            },
            project_view: ProjectViewState {
                projects_state: ListState::default(),
                project_stats: None,
                podcasts: Vec::new(),
                podcasts_state: ListState::default(),
                timelines: Vec::new(),
                timelines_state: ListState::default(),
                builds: Vec::new(),
                builds_state: ListState::default(),
                deploys: Vec::new(),
                deploys_state: ListState::default(),
                sel_tab: ProjectTab::Overview,
            },
            command_palette: CommandPaletteState {
                search: String::new(),
                list_state: ListState::default(),
            },
            local_expanded: true,
            archived_expanded: true,
        }
    }

    async fn load_db_projects() -> Vec<(String, String)> {
        if let Ok(db) = DbManager::open().await {
            db.list_db_projects().await.unwrap_or_default()
        } else {
            vec![]
        }
    }

    async fn refresh_projects(&mut self) {
        self.roots = project::discover_projects(&self.cwd);
        if let Some(sel) = self.projects_state.selected() {
            self.projects_state
                .select(Some(sel.min(self.roots.len().saturating_sub(1))));
        }
        self.db_projects = Self::load_db_projects().await;
    }

    fn selected_root(&self) -> Option<PathBuf> {
        let sel = self.projects_state.selected()?;
        match project_item_type_at(
            sel,
            &self.roots,
            &self.db_projects,
            self.local_expanded,
            self.archived_expanded,
        ) {
            ProjectListItemType::LocalProject(i) => self.roots.get(i).cloned(),
            _ => None,
        }
    }

    fn focus_order(&self) -> Vec<Focus> {
        let sel = self.cmds_state.selected().unwrap_or(0);
        let has_args = !CMDS[sel].args_hint.is_empty();
        let mut order = vec![Focus::Projects, Focus::Commands];
        if has_args {
            order.push(Focus::Details);
        }
        order.push(Focus::Logs);
        order
    }

    fn filtered_palette_commands(&self) -> Vec<usize> {
        let query = self.command_palette.search.to_lowercase();
        CMDS.iter()
            .enumerate()
            .filter(|(_, cmd)| {
                query.is_empty()
                    || cmd.label.to_lowercase().contains(&query)
                    || cmd.desc.to_lowercase().contains(&query)
            })
            .map(|(i, _)| i)
            .collect()
    }

    async fn load_analytics_data(&mut self) {
        let sel = self.analytics.projects_state.selected().unwrap_or(0);
        if let Some(root) = self.roots.get(sel) {
            let project_id = root.to_string_lossy().to_string();
            if let Ok(db) = DbManager::open().await {
                self.analytics.stats = db.get_project_stats(&project_id).await.ok();
            }
        }
        if let Ok(db) = DbManager::open().await {
            self.analytics.global = db.get_all_stats().await.ok();
        }
    }

    pub async fn handle_key(&mut self, key: KeyEvent) {
        if (key.code == KeyCode::Char('p') || key.code == KeyCode::Char('P'))
            && (key.modifiers.contains(KeyModifiers::SUPER)
                || key.modifiers.contains(KeyModifiers::CONTROL))
        {
            if self.mode == TuiMode::CommandPalette {
                self.mode = TuiMode::Runner;
            } else {
                self.mode = TuiMode::CommandPalette;
                self.command_palette.search.clear();
                self.command_palette.list_state.select(Some(0));
            }
            return;
        }

        match self.mode {
            TuiMode::Runner => self.handle_runner_key(key).await,
            TuiMode::Analytics => self.handle_analytics_key(key).await,
            TuiMode::ProjectView => self.handle_project_view_key(key).await,
            TuiMode::CommandPalette => self.handle_command_palette_key(key),
        }
    }

    async fn enter_analytics(&mut self) {
        let mut projects_state = ListState::default();
        if !self.roots.is_empty() {
            projects_state.select(Some(0));
        }
        self.analytics = AnalyticsState {
            projects_state,
            stats: None,
            global: None,
        };
        self.load_analytics_data().await;
        self.mode = TuiMode::Analytics;
        self.focus = Focus::Projects;
    }

    async fn enter_project_view(&mut self) {
        let mut projects_state = ListState::default();
        if !self.roots.is_empty() {
            projects_state.select(Some(0));
        }

        let mut state = ProjectViewState {
            projects_state,
            project_stats: None,
            podcasts: Vec::new(),
            podcasts_state: ListState::default(),
            timelines: Vec::new(),
            timelines_state: ListState::default(),
            builds: Vec::new(),
            builds_state: ListState::default(),
            deploys: Vec::new(),
            deploys_state: ListState::default(),
            sel_tab: ProjectTab::Overview,
        };

        state.load_selected(&self.roots).await;
        self.project_view = state;
        self.mode = TuiMode::ProjectView;
        self.focus = Focus::Projects;
    }

    async fn handle_runner_key(&mut self, key: KeyEvent) {
        if self.editor_pick.is_some() {
            self.handle_pick_key(key);
            return;
        }

        match key.code {
            KeyCode::Char('m') => self.mode = TuiMode::Runner,
            KeyCode::Char('s') => self.enter_analytics().await,
            KeyCode::Char('e') => self.enter_project_view().await,
            _ => self.handle_runner_nav(key).await,
        }
    }

    async fn handle_runner_nav(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Tab | KeyCode::BackTab => {
                if self.busy {
                    return;
                }
                let order = self.focus_order();
                let i = order.iter().position(|f| *f == self.focus).unwrap_or(0);
                let n = order.len();
                self.focus = if key.code == KeyCode::Tab {
                    order[(i + 1) % n]
                } else {
                    order[(i + n - 1) % n]
                };
            }
            KeyCode::Up => match self.focus {
                Focus::Projects if !self.busy => self.projects_state.select_previous(),
                Focus::Logs => self.scroll = self.scroll.saturating_sub(1),
                _ => {}
            },
            KeyCode::Down => match self.focus {
                Focus::Projects if !self.busy => self.projects_state.select_next(),
                Focus::Logs => self.scroll = self.scroll.saturating_add(1),
                _ => {}
            },
            KeyCode::Left => match self.focus {
                Focus::Commands if !self.busy => {
                    let sel = self.cmds_state.selected().unwrap_or(0);
                    if sel > 0 {
                        self.cmds_state.select(Some(sel - 1));
                    }
                }
                _ => {}
            },
            KeyCode::Right => match self.focus {
                Focus::Commands if !self.busy => {
                    let sel = self.cmds_state.selected().unwrap_or(0);
                    if sel + 1 < CMDS.len() {
                        self.cmds_state.select(Some(sel + 1));
                    }
                }
                _ => {}
            },
            KeyCode::Enter => {
                if !self.busy {
                    match self.focus {
                        Focus::Projects => {
                            let Some(sel) = self.projects_state.selected() else {
                                return;
                            };
                            let item_type = project_item_type_at(
                                sel,
                                &self.roots,
                                &self.db_projects,
                                self.local_expanded,
                                self.archived_expanded,
                            );
                            match item_type {
                                ProjectListItemType::LocalHeader => {
                                    self.local_expanded = !self.local_expanded;
                                    self.projects_state.select(Some(0));
                                }
                                ProjectListItemType::ArchivedHeader => {
                                    self.archived_expanded = !self.archived_expanded;
                                    let idx =
                                        archived_header_index(&self.roots, self.local_expanded);
                                    self.projects_state.select(Some(idx));
                                }
                                ProjectListItemType::LocalProject(_) => {
                                    self.open_edit_picker();
                                }
                                ProjectListItemType::ArchivedProject => {}
                            }
                        }
                        Focus::Commands | Focus::Details => self.start_cmd(),
                        _ => {}
                    }
                }
            }
            KeyCode::Backspace => {
                if matches!(self.focus, Focus::Details) && !self.busy {
                    self.arg_input.pop();
                }
            }
            KeyCode::Char('r') if !matches!(self.focus, Focus::Details) => {
                if !self.busy {
                    self.refresh_projects().await;
                    self.log.clear();
                    self.scroll = 0;
                    info!(">> Refreshed");
                }
            }
            KeyCode::Char(ch) => {
                if matches!(self.focus, Focus::Details) && !self.busy {
                    self.arg_input.push(ch);
                }
            }
            KeyCode::PageUp if matches!(self.focus, Focus::Logs) => {
                self.scroll = self.scroll.saturating_sub(10)
            }
            KeyCode::PageDown if matches!(self.focus, Focus::Logs) => {
                self.scroll = self.scroll.saturating_add(10)
            }
            _ => {}
        }
    }

    async fn handle_analytics_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('m') => {
                self.mode = TuiMode::Runner;
                self.focus = Focus::Commands;
            }
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = if self.focus == Focus::Projects {
                    Focus::Details
                } else {
                    Focus::Projects
                };
            }
            KeyCode::Up => {
                if self.focus == Focus::Projects {
                    self.analytics.projects_state.select_previous();
                    self.load_analytics_data().await;
                }
            }
            KeyCode::Down => {
                if self.focus == Focus::Projects {
                    self.analytics.projects_state.select_next();
                    self.load_analytics_data().await;
                }
            }
            KeyCode::Char('r') => {
                self.analytics.stats = None;
                self.analytics.global = None;
                self.load_analytics_data().await;
                info!(">> Analytics refreshed");
            }
            _ => {}
        }
    }

    async fn handle_project_view_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('m') => {
                self.mode = TuiMode::Runner;
                self.focus = Focus::Commands;
            }
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = if self.focus == Focus::Projects {
                    Focus::Details
                } else {
                    Focus::Projects
                };
            }
            KeyCode::Up => {
                if self.focus == Focus::Projects {
                    self.project_view.projects_state.select_previous();
                    self.project_view.load_selected(&self.roots).await;
                } else if self.focus == Focus::Details {
                    match self.project_view.sel_tab {
                        ProjectTab::Podcasts => self.project_view.podcasts_state.select_previous(),
                        ProjectTab::Timelines => {
                            self.project_view.timelines_state.select_previous()
                        }
                        ProjectTab::Builds => self.project_view.builds_state.select_previous(),
                        ProjectTab::Deployments => {
                            self.project_view.deploys_state.select_previous()
                        }
                        _ => {}
                    }
                }
            }
            KeyCode::Down => {
                if self.focus == Focus::Projects {
                    self.project_view.projects_state.select_next();
                    self.project_view.load_selected(&self.roots).await;
                } else if self.focus == Focus::Details {
                    match self.project_view.sel_tab {
                        ProjectTab::Podcasts => self.project_view.podcasts_state.select_next(),
                        ProjectTab::Timelines => self.project_view.timelines_state.select_next(),
                        ProjectTab::Builds => self.project_view.builds_state.select_next(),
                        ProjectTab::Deployments => self.project_view.deploys_state.select_next(),
                        _ => {}
                    }
                }
            }
            KeyCode::Left | KeyCode::Right => {
                if self.focus == Focus::Details {
                    let tabs = [
                        ProjectTab::Overview,
                        ProjectTab::Podcasts,
                        ProjectTab::Timelines,
                        ProjectTab::Builds,
                        ProjectTab::Deployments,
                    ];
                    let idx = tabs
                        .iter()
                        .position(|t| *t == self.project_view.sel_tab)
                        .unwrap_or(0);
                    let new_idx = if key.code == KeyCode::Left {
                        (idx + tabs.len() - 1) % tabs.len()
                    } else {
                        (idx + 1) % tabs.len()
                    };
                    self.project_view.sel_tab = tabs[new_idx];
                }
            }
            KeyCode::Char('r') => self.project_view.load_selected(&self.roots).await,
            _ => {}
        }
    }

    fn handle_command_palette_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.mode = TuiMode::Runner,
            KeyCode::Up => self.command_palette.list_state.select_previous(),
            KeyCode::Down => self.command_palette.list_state.select_next(),
            KeyCode::Enter => {
                let filtered = self.filtered_palette_commands();
                if let Some(&cmd_idx) = self
                    .command_palette
                    .list_state
                    .selected()
                    .and_then(|i| filtered.get(i))
                {
                    self.cmds_state.select(Some(cmd_idx));
                    self.mode = TuiMode::Runner;
                    self.focus = Focus::Commands;
                    self.start_cmd();
                }
            }
            KeyCode::Backspace => {
                self.command_palette.search.pop();
                self.command_palette.list_state.select(Some(0));
            }
            KeyCode::Char(c) => {
                self.command_palette.search.push(c);
                self.command_palette.list_state.select(Some(0));
            }
            _ => {}
        }
    }

    fn handle_pick_key(&mut self, key: KeyEvent) {
        let Some(pick) = self.editor_pick.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Up => pick.state.select_previous(),
            KeyCode::Down => pick.state.select_next(),
            KeyCode::Enter => {
                if let Some(idx) = pick.state.selected() {
                    self.pending_edit = pick.files.get(idx).cloned();
                    self.editor_pick = None;
                }
            }
            KeyCode::Esc | KeyCode::Char('q') => self.editor_pick = None,
            _ => {}
        }
    }

    fn open_edit_picker(&mut self) {
        let Some(root) = self.selected_root() else {
            error!("No project selected");
            return;
        };
        let metadata_file = ProjectContext::load(&root)
            .map(|c| c.manifest.project.metadata_file)
            .unwrap_or_else(|_| "metadata.yml".into());
        let files = vec![root.join("cite.toml"), root.join(metadata_file)];
        let mut state = ListState::default();
        state.select(Some(0));
        self.editor_pick = Some(EditorPick { files, state });
    }

    fn start_cmd(&mut self) {
        let root = self.selected_root();
        let sel = self.cmds_state.selected().unwrap_or(0);
        let cmd = &CMDS[sel];

        if cmd.needs_project && root.is_none() {
            error!("No projects found; select or init a project first");
            return;
        }

        let raw_args = std::mem::take(&mut self.arg_input);
        let arg_display = if raw_args.is_empty() {
            String::new()
        } else {
            format!(" ({})", raw_args)
        };
        info!(
            ">> {}{} {}",
            cmd.label,
            arg_display,
            root.as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        );

        self.busy = true;
        let id = cmd.id;
        let tx = self.tx.clone();
        let cwd = self.cwd.clone();

        let handle = tokio::spawn(async move {
            match id {
                CommandId::Init => exec_init(cwd, raw_args).await,
                CommandId::Build => exec_build(root, raw_args).await,
                CommandId::Lint => exec_lint(root, raw_args).await,
                CommandId::Status => exec_status(root, raw_args).await,
                CommandId::Doctor => exec_doctor(root, raw_args).await,
                CommandId::Deploy => exec_deploy(root, raw_args).await,
                CommandId::Rollback => exec_rollback(root, raw_args).await,
                CommandId::Clean => exec_clean(root, raw_args).await,
            }
            let _ = tx.send(()).await;
        });
        self.task = Some(handle);
    }
}

impl ProjectViewState {
    async fn load_selected(&mut self, roots: &[PathBuf]) {
        let sel = self.projects_state.selected().unwrap_or(0);
        if let Some(root) = roots.get(sel) {
            let project_id = root.to_string_lossy().to_string();
            if let Ok(db) = DbManager::open().await {
                self.project_stats = db.get_project_stats(&project_id).await.ok();
                self.podcasts = db
                    .get_podcasts_with_content(&project_id)
                    .await
                    .ok()
                    .unwrap_or_default();
                self.timelines = db.get_timelines(&project_id).await.ok().unwrap_or_default();
                self.builds = db
                    .get_build_history(&project_id)
                    .await
                    .ok()
                    .unwrap_or_default();
                self.deploys = db
                    .get_deployment_history(&project_id)
                    .await
                    .ok()
                    .unwrap_or_default();
            }
        }
    }
}

// --- Rendering ---

fn block(title: impl Into<String>, focused: bool) -> Block<'static> {
    let border_style = if focused {
        Style::new().fg(Color::Cyan)
    } else {
        Style::new()
    };
    Block::default()
        .borders(Borders::ALL)
        .title(title.into())
        .border_style(border_style)
}

fn color_log_line(l: &str) -> Line<'static> {
    if l.contains("ERROR") {
        Line::styled(l.to_string(), Style::new().fg(Color::Red).bold())
    } else if l.contains("WARN") {
        Line::styled(l.to_string(), Style::new().fg(Color::Yellow))
    } else {
        Line::from(l.to_string())
    }
}

pub async fn run_tui(
    mut log_rx: mpsc::UnboundedReceiver<String>,
    cli_root: PathBuf,
) -> Result<(), CiteError> {
    let mut terminal = ratatui::init();
    let _guard = TerminalGuard;
    terminal
        .clear()
        .map_err(|e| CiteError::Config(format!("{e}")))?;

    let mut app = AppState::new(&cli_root).await;

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<Event>();
    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);

    let event_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => break,
                result = tokio::task::spawn_blocking(|| {
                    if event::poll(std::time::Duration::from_millis(100)).unwrap_or(false) { event::read().ok() } else { None }
                }) => {
                    if let Ok(Some(event)) = result && event_tx.send(event).is_err() { break; }
                }
            }
        }
    });

    loop {
        terminal
            .draw(|f| render(f, &mut app))
            .map_err(|e| CiteError::Config(format!("{e}")))?;

        tokio::select! {
            biased;
            Some(()) = app.rx.recv() => { app.busy = false; app.task = None; app.refresh_projects().await; }
            Some(line) = log_rx.recv() => {
                let was_at_bottom = app.scroll >= app.log.len().saturating_sub(1);
                app.log.push(line);
                if was_at_bottom || app.log.len() <= 1 { app.scroll = app.log.len().saturating_sub(1); }
            }
            Some(event) = event_rx.recv() => {
                if let Event::Key(key) = event
                    && key.kind == KeyEventKind::Press {
                        if (key.code == KeyCode::Char('q') && key.modifiers.is_empty() && app.mode == TuiMode::Runner)
                            || (key.code == KeyCode::Esc && app.mode == TuiMode::Runner && app.editor_pick.is_none()) { break; }
                        app.handle_key(key).await;

                        if let Some(path) = app.pending_edit.take() {
                            edit_file(&mut terminal, &mut app, &path).await.map_err(|e| CiteError::Config(format!("{e}")))?;
                        }
                    }
            }
        }
    }

    if let Some(task) = app.task.take() {
        task.abort();
    }
    let _ = shutdown_tx.send(()).await;
    let _ = event_task.await;
    Ok(())
}

fn render(frame: &mut Frame, app: &mut AppState) {
    let [header, body, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    render_header(frame, header, app);
    render_body(frame, body, app);
    render_statusbar(frame, status, app);

    if app.editor_pick.is_some() {
        render_editor_pick(frame, frame.area(), app);
    }

    if app.mode == TuiMode::CommandPalette {
        render_command_palette(frame, frame.area(), app);
    }
}

fn render_header(frame: &mut Frame, area: Rect, app: &AppState) {
    let left_text = if app.busy {
        let cmd = &CMDS[app.cmds_state.selected().unwrap_or(0)];
        format!("Running: {} on {}", cmd.label, app.cwd.display())
    } else {
        "Ready".to_string()
    };

    let style = if app.busy {
        Style::new().fg(Color::Yellow)
    } else {
        Style::new()
    };
    let version = Span::styled(format!("v{}", env!("CARGO_PKG_VERSION")), Style::new());

    let [left_area, right_area] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(10)]).areas(area);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(left_text, style))),
        left_area,
    );
    frame.render_widget(
        Paragraph::new(Line::from(version)).alignment(Alignment::Right),
        right_area,
    );
}

fn render_body(frame: &mut Frame, area: Rect, app: &mut AppState) {
    match app.mode {
        TuiMode::Runner | TuiMode::CommandPalette => {
            let [left, right] =
                Layout::horizontal([Constraint::Percentage(20), Constraint::Percentage(80)])
                    .areas(area);
            render_categorized_project_list(
                frame,
                left,
                &app.roots,
                &app.db_projects,
                app.local_expanded,
                app.archived_expanded,
                &app.focus,
                &mut app.projects_state,
            );
            let [content_area, logs_area] =
                Layout::vertical([Constraint::Fill(1), Constraint::Percentage(35)]).areas(right);
            let [cmd_tabs_area, details_area] =
                Layout::vertical([Constraint::Length(3), Constraint::Fill(1)]).areas(content_area);
            render_cmd_tabs(frame, cmd_tabs_area, app);
            render_cmd_doc(frame, details_area, app);
            render_log(frame, logs_area, app);
        }
        TuiMode::Analytics => {
            let [left, right] =
                Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(70)])
                    .areas(area);
            render_simple_project_list(
                frame,
                left,
                &app.roots,
                &app.focus,
                &mut app.analytics.projects_state,
            );
            render_analytics_content(frame, right, app);
        }
        TuiMode::ProjectView => {
            let [left, right] =
                Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(70)])
                    .areas(area);
            render_simple_project_list(
                frame,
                left,
                &app.roots,
                &app.focus,
                &mut app.project_view.projects_state,
            );
            render_explorer_content(frame, right, app);
        }
    }
}

fn compute_archived<'a>(roots: &[PathBuf], db_projects: &'a [(String, String)]) -> Vec<&'a str> {
    db_projects
        .iter()
        .filter(|(name, id)| {
            !roots.iter().any(|r| {
                r.to_string_lossy().as_ref() == id.as_str()
                    || r.file_name().and_then(|n| n.to_str()) == Some(name.as_str())
            })
        })
        .map(|(name, _)| name.as_str())
        .collect()
}

fn project_item_type_at(
    sel: usize,
    roots: &[PathBuf],
    db_projects: &[(String, String)],
    local_expanded: bool,
    archived_expanded: bool,
) -> ProjectListItemType {
    if sel == 0 {
        return ProjectListItemType::LocalHeader;
    }

    let mut cursor = 1;

    if local_expanded {
        if roots.is_empty() {
            if sel == cursor {
                return ProjectListItemType::ArchivedProject;
            }
            cursor += 1;
        } else {
            let n = roots.len();
            if sel < cursor + n {
                return ProjectListItemType::LocalProject(sel - cursor);
            }
            cursor += n;
        }
    }

    if sel == cursor {
        return ProjectListItemType::ArchivedHeader;
    }
    cursor += 1;

    if archived_expanded {
        let archived = compute_archived(roots, db_projects);
        if archived.is_empty() {
            if sel == cursor {
                return ProjectListItemType::ArchivedProject;
            }
        } else if sel < cursor + archived.len() {
            return ProjectListItemType::ArchivedProject;
        }
    }

    ProjectListItemType::ArchivedProject
}

fn archived_header_index(roots: &[PathBuf], local_expanded: bool) -> usize {
    let mut idx = 1;
    if local_expanded {
        idx += if roots.is_empty() { 1 } else { roots.len() };
    }
    idx
}

fn render_categorized_project_list(
    frame: &mut Frame,
    area: Rect,
    roots: &[PathBuf],
    db_projects: &[(String, String)],
    local_expanded: bool,
    archived_expanded: bool,
    focus: &Focus,
    state: &mut ListState,
) {
    let is_focused = matches!(focus, Focus::Projects);

    let header_style = if is_focused {
        Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::new().add_modifier(Modifier::BOLD)
    };

    let mut items: Vec<ListItem> = Vec::new();

    let local_indicator = if local_expanded { "▼" } else { "▶" };
    items.push(ListItem::new(Line::from(Span::styled(
        format!(" {} Local", local_indicator),
        header_style,
    ))));
    if local_expanded {
        if roots.is_empty() {
            items.push(ListItem::new("  (none)"));
        } else {
            for root in roots {
                let name = root.file_name().and_then(|n| n.to_str()).unwrap_or("?");
                items.push(ListItem::new(format!("  {}  ", name)));
            }
        }
    }

    let archived_indicator = if archived_expanded { "▼" } else { "▶" };
    items.push(ListItem::new(Line::from(Span::styled(
        format!(" {} Archived", archived_indicator),
        header_style,
    ))));
    if archived_expanded {
        let archived_names = compute_archived(roots, db_projects);
        if archived_names.is_empty() {
            items.push(ListItem::new("  (none)"));
        } else {
            for name in archived_names {
                items.push(ListItem::new(format!("  {}  ", name)));
            }
        }
    }

    if let Some(sel) = state.selected() {
        if sel >= items.len() {
            if items.is_empty() {
                state.select(None);
            } else {
                state.select(Some(items.len().saturating_sub(1)));
            }
        }
    } else if !items.is_empty() {
        state.select(Some(0));
    }

    let list = List::new(items)
        .block(block(" Projects ", is_focused))
        .highlight_style(Style::new().bold())
        .highlight_symbol("▸ ");
    frame.render_stateful_widget(list, area, state);
}

fn render_simple_project_list(
    frame: &mut Frame,
    area: Rect,
    roots: &[PathBuf],
    focus: &Focus,
    state: &mut ListState,
) {
    let is_focused = matches!(focus, Focus::Projects);

    let mut items: Vec<ListItem> = Vec::new();
    if roots.is_empty() {
        items.push(ListItem::new("  (none)"));
    } else {
        for root in roots {
            let name = root.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            items.push(ListItem::new(format!("  {}  ", name)));
        }
    }

    if let Some(sel) = state.selected() {
        if sel >= items.len() {
            if items.is_empty() {
                state.select(None);
            } else {
                state.select(Some(items.len().saturating_sub(1)));
            }
        }
    } else if !items.is_empty() {
        state.select(Some(0));
    }

    let list = List::new(items)
        .block(block(" Projects ", is_focused))
        .highlight_style(Style::new().bold())
        .highlight_symbol("▸ ");
    frame.render_stateful_widget(list, area, state);
}

fn render_cmd_tabs(frame: &mut Frame, area: Rect, app: &mut AppState) {
    let is_focused = matches!(app.focus, Focus::Commands);
    let titles: Vec<Line> = CMDS.iter().map(|cmd| Line::from(cmd.label)).collect();
    let border_style = if is_focused {
        Style::new().fg(Color::Cyan)
    } else {
        Style::new()
    };

    let tabs = Tabs::new(titles)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Commands ")
                .border_style(border_style),
        )
        .select(app.cmds_state.selected().unwrap_or(0))
        .divider(symbols::DOT)
        .highlight_style(Style::new().bold().fg(Color::Cyan));

    frame.render_widget(tabs, area);
}

fn render_cmd_doc(frame: &mut Frame, area: Rect, app: &AppState) {
    let sel = app.cmds_state.selected().unwrap_or(0);
    let cmd = &CMDS[sel];
    let is_focused = matches!(app.focus, Focus::Details);
    let has_args = !cmd.args_hint.is_empty();

    let mut lines = vec![
        Line::from(Span::styled(cmd.label, Style::new().bold())),
        Line::from(""),
        Line::from(cmd.desc),
    ];

    if has_args {
        lines.extend_from_slice(&[
            Line::from(""),
            Line::from(Span::styled("Arguments:", Style::new().bold())),
            Line::from(Span::raw(format!("  {}", cmd.args_hint))),
            Line::from(""),
        ]);
        let input_text = if app.arg_input.is_empty() {
            "Awaiting input...|".to_string()
        } else {
            format!("{}|", app.arg_input)
        };
        let cursor_style = if is_focused {
            Style::new().bold().add_modifier(Modifier::UNDERLINED)
        } else {
            Style::new()
        };
        lines.push(Line::from(vec![
            Span::raw("Input: "),
            Span::styled(input_text, cursor_style),
        ]));
    } else {
        lines.extend_from_slice(&[Line::from(""), Line::from("No arguments required")]);
    }

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block(" Details ", is_focused))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_log(frame: &mut Frame, area: Rect, app: &AppState) {
    let is_focused = matches!(app.focus, Focus::Logs);
    let visible_lines = area.height.saturating_sub(2) as usize;
    let max_scroll = app.log.len().saturating_sub(visible_lines);
    let scroll_y = app.scroll.min(max_scroll);
    let end = (scroll_y + visible_lines).min(app.log.len());

    let lines: Vec<Line> = app.log[scroll_y..end]
        .iter()
        .map(|l| color_log_line(l))
        .collect();
    let block_widget = block(" Logs ", is_focused);
    let inner_area = block_widget.inner(area);

    frame.render_widget(block_widget, area);
    frame.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
        inner_area,
    );

    if app.log.len() > visible_lines {
        let mut scroll_state = ScrollbarState::default()
            .content_length(app.log.len())
            .position(scroll_y);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None),
            area,
            &mut scroll_state,
        );
    }
}

fn render_statusbar(frame: &mut Frame, area: Rect, app: &AppState) {
    let (mode_label, help_text) = match app.mode {
        TuiMode::Runner | TuiMode::CommandPalette => (
            " Runner ",
            "[m] Main  [s] Analytics  [e] Explorer  [tab] Cycle  [q] Quit",
        ),
        TuiMode::Analytics => (
            " Analytics ",
            "[tab] Switch Panels  [↑/↓] Navigate  [r] Refresh  [esc/m] Back",
        ),
        TuiMode::ProjectView => (
            " Explorer ",
            "[tab] Switch Panels  [←/→] Tabs  [↑/↓] Nav  [r] Refresh  [esc/m] Back",
        ),
    };

    let [left_area, right_area] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Fill(1)]).areas(area);

    let left_style = Style::new().bold().fg(Color::Cyan);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(mode_label, left_style))),
        left_area,
    );
    frame.render_widget(
        Paragraph::new(Line::from(help_text)).alignment(Alignment::Right),
        right_area,
    );
}

fn render_command_palette(frame: &mut Frame, area: Rect, app: &mut AppState) {
    let width = 70u16.min(area.width.saturating_sub(4));
    let height = 15u16.min(area.height.saturating_sub(4));
    let [_, mid, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(height),
        Constraint::Fill(1),
    ])
    .areas(area);
    let [_, popup, _] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(width),
        Constraint::Fill(1),
    ])
    .areas(mid);

    frame.render_widget(Clear, popup);
    let filtered = app.filtered_palette_commands();

    let items: Vec<ListItem> = filtered
        .iter()
        .map(|&cmd_idx| {
            let cmd = &CMDS[cmd_idx];
            ListItem::new(format!("{}: {}", cmd.label, cmd.desc))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(Style::new().bold())
        .highlight_symbol("▸ ");
    let palette_block = Block::default()
        .borders(Borders::ALL)
        .title(" Command Palette (Ctrl+P) ")
        .border_style(Style::new().fg(Color::Cyan));

    let inner = palette_block.inner(popup);
    frame.render_widget(palette_block, popup);

    let search_text = format!("> {}|", app.command_palette.search);
    let [search_area, list_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(inner);
    frame.render_widget(Paragraph::new(search_text), search_area);
    frame.render_stateful_widget(list, list_area, &mut app.command_palette.list_state);
}

fn render_editor_pick(frame: &mut Frame, area: Rect, app: &mut AppState) {
    let Some(pick) = &mut app.editor_pick else {
        return;
    };
    let width = 60u16.min(area.width.saturating_sub(2));
    let height = (pick.files.len() as u16 + 4).min(area.height.saturating_sub(2));
    let [_, mid, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(height),
        Constraint::Fill(1),
    ])
    .areas(area);
    let [_, popup, _] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(width),
        Constraint::Fill(1),
    ])
    .areas(mid);

    let items: Vec<ListItem> = pick
        .files
        .iter()
        .map(|f| ListItem::new(f.file_name().and_then(|n| n.to_str()).unwrap_or("?")))
        .collect();
    let list = List::new(items)
        .block(block(" Select File to Edit ", true))
        .highlight_style(Style::new().bold())
        .highlight_symbol("▸ ");

    frame.render_widget(Clear, popup);
    frame.render_stateful_widget(list, popup, &mut pick.state);
}

fn render_analytics_content(frame: &mut Frame, area: Rect, app: &AppState) {
    let is_focused = matches!(app.focus, Focus::Details);
    let mut lines: Vec<Line> = Vec::new();

    // Global summary
    if let Some(ref global) = app.analytics.global {
        lines.push(Line::from(Span::styled(
            "Global Summary",
            Style::new().bold(),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(format!(
            "Projects: {}  Podcasts: {}  Timelines: {}  Words: {}  Builds: {}",
            global.project_count,
            global.total_podcasts,
            global.total_timelines,
            global.total_words,
            global.total_builds,
        )));
        lines.push(Line::from(""));
    }

    // Per-project stats
    if let Some(ref stats) = app.analytics.stats {
        lines.push(Line::from(Span::styled(
            "Project Statistics",
            Style::new().bold(),
        )));
        lines.push(Line::from(""));

        let project_name = app
            .roots
            .get(app.analytics.projects_state.selected().unwrap_or(0))
            .and_then(|p| p.file_name().and_then(|n| n.to_str()))
            .unwrap_or("(none selected)");
        lines.push(Line::from(format!("Project: {}", project_name)));
        lines.push(Line::from(""));
        lines.push(Line::from(format!("Podcasts:     {}", stats.podcast_count)));
        lines.push(Line::from(format!("Total Words:  {}", stats.total_words)));

        let reading_time = if stats.total_words > 0 {
            format!(
                "{} min (est.)",
                (stats.total_words as f64 / 200.0).ceil() as u64
            )
        } else {
            "N/A".to_string()
        };
        lines.push(Line::from(format!("Reading Time: {}", reading_time)));

        lines.push(Line::from(format!(
            "Timelines:    {}",
            stats.timeline_count
        )));

        // Build info
        lines.push(Line::from(format!("Builds:       {}", stats.build_count)));
        if let Some(ref last) = stats.last_built {
            let d = last.get(..19).unwrap_or(last);
            lines.push(Line::from(format!("Last Build:   {}", d)));
        }

        // Deployment info
        lines.push(Line::from(format!(
            "Deployments:  {}",
            stats.deployment_count
        )));
        if let Some(ref last) = stats.last_deployed {
            let d = last.get(..19).unwrap_or(last);
            lines.push(Line::from(format!("Last Deploy:  {}", d)));
        }

        if let Some(global) = &app.analytics.global {
            lines.push(Line::from(format!(
                "Total Assets: {}",
                global.total_podcasts
            )));
        }

        // Podcasts by month
        if !stats.podcasts_by_month.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Builds by Month",
                Style::new().bold(),
            )));
            for (m, c) in &stats.podcasts_by_month {
                let bar_width = area.width.saturating_sub(25) as usize;
                let bar = "█".repeat((*c as usize).min(bar_width));
                lines.push(Line::from(format!("  {:<10} {:>3}  {}", m, c, bar)));
            }
        }

        // Citations by decade
        if !stats.citations_by_decade.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Citations by Decade",
                Style::new().bold(),
            )));
            for (d, c) in &stats.citations_by_decade {
                let bar_width = area.width.saturating_sub(25) as usize;
                let bar = "█".repeat((*c as usize).min(bar_width));
                lines.push(Line::from(format!("  {:<10} {:>3}  {}", d, c, bar)));
            }
        }
    } else {
        lines.push(Line::from(""));
        lines.push(Line::from("Select a project to view analytics"));
    }

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block(" Analytics ", is_focused))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_explorer_content(frame: &mut Frame, area: Rect, app: &mut AppState) {
    let pv = &mut app.project_view;
    let [tabs_area, list_area] =
        Layout::vertical([Constraint::Length(3), Constraint::Fill(1)]).areas(area);

    let project_name = app
        .roots
        .get(pv.projects_state.selected().unwrap_or(0))
        .and_then(|p| p.file_name().and_then(|n| n.to_str()))
        .unwrap_or("(no projects)");
    let titles: Vec<Line> = vec!["Overview", "Podcasts", "Timelines", "Builds", "Deployments"]
        .into_iter()
        .map(Line::from)
        .collect();

    let tabs_is_focused = matches!(app.focus, Focus::Details);

    let tabs = Tabs::new(titles)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Project: {} ", project_name))
                .border_style(if tabs_is_focused {
                    Style::new().fg(Color::Cyan)
                } else {
                    Style::new()
                }),
        )
        .divider(symbols::DOT)
        .select(pv.sel_tab as usize)
        .highlight_style(Style::new().bold().fg(Color::Cyan));
    frame.render_widget(tabs, tabs_area);

    let inner = list_area.width.saturating_sub(2) as usize;

    match pv.sel_tab {
        ProjectTab::Overview => {
            let lines = if let Some(ref stats) = pv.project_stats {
                let rt = if stats.total_words > 0 {
                    format!("{} min", (stats.total_words as f64 / 200.0).ceil() as u64)
                } else {
                    "N/A".to_string()
                };
                let mut v = vec![
                    Line::from("Project Statistics"),
                    Line::from(""),
                    Line::from(format!("  Podcasts:     {}", stats.podcast_count)),
                    Line::from(format!("  Timelines:    {}", stats.timeline_count)),
                    Line::from(format!("  Total Words:  {}", stats.total_words)),
                    Line::from(format!("  Reading Time: {}", rt)),
                    Line::from(format!("  Builds:       {}", stats.build_count)),
                    Line::from(format!("  Deployments:  {}", stats.deployment_count)),
                ];
                if let Some(ref last) = stats.last_built {
                    let d = last.get(..19).unwrap_or(last);
                    v.push(Line::from(format!("  Last Build:   {}", d)));
                } else {
                    v.push(Line::from("  Last Build:   never"));
                }
                if let Some(ref last) = stats.last_deployed {
                    let d = last.get(..19).unwrap_or(last);
                    v.push(Line::from(format!("  Last Deploy:  {}", d)));
                } else {
                    v.push(Line::from("  Last Deploy:  never"));
                }
                // Podcasts by month
                if !stats.podcasts_by_month.is_empty() {
                    v.push(Line::from(""));
                    v.push(Line::from("Builds by Month"));
                    for (m, c) in &stats.podcasts_by_month {
                        let bar = "█".repeat((*c as usize).min(inner.saturating_sub(15)));
                        v.push(Line::from(format!("  {:<10} {:>3} {}", m, c, bar)));
                    }
                }
                // Citations by decade
                if !stats.citations_by_decade.is_empty() {
                    v.push(Line::from(""));
                    v.push(Line::from("Citations by Decade"));
                    for (d, c) in &stats.citations_by_decade {
                        let bar = "█".repeat((*c as usize).min(inner.saturating_sub(15)));
                        v.push(Line::from(format!("  {:<10} {:>3} {}", d, c, bar)));
                    }
                }
                v
            } else {
                vec![Line::from("No data available")]
            };
            frame.render_widget(
                Paragraph::new(Text::from(lines))
                    .block(block("Overview", false))
                    .wrap(Wrap { trim: false }),
                list_area,
            );
        }
        ProjectTab::Podcasts => {
            let items: Vec<ListItem> = pv
                .podcasts
                .iter()
                .map(|p| ListItem::new(format!("{}  ({} words)", p.title, p.word_count)))
                .collect();
            let list = List::new(items)
                .block(block("Podcasts", false))
                .highlight_style(Style::new().bold())
                .highlight_symbol("▸ ");
            frame.render_stateful_widget(list, list_area, &mut pv.podcasts_state);
        }
        ProjectTab::Timelines => {
            let items: Vec<ListItem> = pv
                .timelines
                .iter()
                .map(|t| {
                    ListItem::new(format!(
                        "{}  {}",
                        t.date.as_deref().unwrap_or("??"),
                        t.title
                    ))
                })
                .collect();
            let list = List::new(items)
                .block(block("Timelines", false))
                .highlight_style(Style::new().bold())
                .highlight_symbol("▸ ");
            frame.render_stateful_widget(list, list_area, &mut pv.timelines_state);
        }
        ProjectTab::Builds => {
            let items: Vec<ListItem> = pv
                .builds
                .iter()
                .map(|b| {
                    let kind = if b.was_incremental { "incr" } else { "full" };
                    ListItem::new(format!(
                        "p:{} t:{} w:{} {:>4}ms ({kind})",
                        b.podcast_count, b.timeline_count, b.total_words, b.duration_ms
                    ))
                })
                .collect();
            if items.is_empty() {
                frame.render_widget(
                    Paragraph::new(Text::from(vec![Line::from(
                        "No builds yet. Run the build command.",
                    )]))
                    .block(block("Build History", false))
                    .wrap(Wrap { trim: false }),
                    list_area,
                );
            } else {
                let list = List::new(items)
                    .block(block("Build History", false))
                    .highlight_style(Style::new().bold())
                    .highlight_symbol("▸ ");
                frame.render_stateful_widget(list, list_area, &mut pv.builds_state);
            }
        }
        ProjectTab::Deployments => {
            let items: Vec<ListItem> = pv
                .deploys
                .iter()
                .map(|d| {
                    let status = if d.success { "OK" } else { "FAIL" };
                    let date = d.deployed_at.get(..19).unwrap_or(&d.deployed_at);
                    ListItem::new(format!("{}  {:.8}  {status}", date, d.deployment_id))
                })
                .collect();
            if items.is_empty() {
                frame.render_widget(
                    Paragraph::new(Text::from(vec![Line::from(
                        "No deployments yet. Run the deploy command.",
                    )]))
                    .block(block("Deployment History", false))
                    .wrap(Wrap { trim: false }),
                    list_area,
                );
            } else {
                let list = List::new(items)
                    .block(block("Deployment History", false))
                    .highlight_style(Style::new().bold())
                    .highlight_symbol("▸ ");
                frame.render_stateful_widget(list, list_area, &mut pv.deploys_state);
            }
        }
    }
}

// --- Utilities & Execution ---

async fn edit_file(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut AppState,
    path: &Path,
) -> std::io::Result<()> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());
    info!(">> Editing {} in {editor}", path.display());
    let before = file_digest(path).await;

    ratatui::restore();
    let editor_clone = editor.clone();
    let path_clone = path.to_path_buf();

    let status = tokio::task::spawn_blocking(move || {
        std::process::Command::new(&editor_clone)
            .arg(&path_clone)
            .status()
    })
    .await
    .map_err(std::io::Error::other)??;

    *terminal = ratatui::init();
    terminal.clear()?;

    match status {
        s if s.success() => {
            if file_digest(path).await != before {
                info!("Edited {}", path.display());
            } else {
                info!("No changes to {}", path.display());
            }
        }
        s => warn!("Editor exited with {s}"),
    }
    app.refresh_projects().await;
    Ok(())
}

async fn file_digest(path: &Path) -> Option<[u8; 32]> {
    use sha2::Digest;
    let bytes = tokio::fs::read(path).await.ok()?;
    Some(sha2::Sha256::digest(&bytes).into())
}

fn resolve_relative(base: &Path, relative: &str) -> PathBuf {
    let base = std::fs::canonicalize(base).unwrap_or_else(|_| base.to_path_buf());
    let mut result = base;
    for comp in Path::new(relative).components() {
        match comp {
            Component::ParentDir => {
                result.pop();
            }
            Component::Normal(c) => {
                result.push(c);
            }
            _ => {}
        }
    }
    result
}

async fn exec_init(cwd: PathBuf, raw: String) {
    let name = raw.split_whitespace().next().unwrap_or("new-project");
    let target = resolve_relative(&cwd, name);
    match scaffold::init_project(name, &target) {
        Ok(_) => info!("Project '{name}' created at {}", target.display()),
        Err(e) => error!("Init failed: {e}"),
    }
}

async fn exec_build(root: Option<PathBuf>, raw: String) {
    let Some(root) = root else {
        error!("No project selected");
        return;
    };
    let ctx = match ProjectContext::load(&root) {
        Ok(ctx) => ctx,
        Err(e) => {
            error!("{e}");
            return;
        }
    };
    let force = raw.split_whitespace().any(|w| w == "--force");
    match compiler::compile(&ctx, force).await {
        Ok(outcome) => outcome.emit(),
        Err(e) => error!("Build failed: {e}"),
    }
}

async fn exec_lint(root: Option<PathBuf>, _raw: String) {
    let Some(root) = root else {
        error!("No project selected");
        return;
    };
    let ctx = match ProjectContext::load(&root) {
        Ok(ctx) => ctx,
        Err(e) => {
            error!("{e}");
            return;
        }
    };
    doctor::lint_all(&ctx).emit();
}

async fn exec_status(root: Option<PathBuf>, _raw: String) {
    let Some(root) = root else {
        error!("No project selected");
        return;
    };
    let ctx = match ProjectContext::load(&root) {
        Ok(ctx) => ctx,
        Err(e) => {
            error!("{e}");
            return;
        }
    };
    project::print_status(&ctx).await;
}

async fn exec_doctor(root: Option<PathBuf>, _raw: String) {
    let Some(root) = root else {
        error!("No project selected");
        return;
    };
    let ctx = match ProjectContext::load(&root) {
        Ok(ctx) => ctx,
        Err(e) => {
            error!("{e}");
            return;
        }
    };
    match doctor::run(&ctx).await {
        Ok(o) => {
            if !o.has_errors() && !o.has_warnings() {
                info!("Doctor check complete; no issues found");
            }
        }
        Err(e) => error!("Doctor failed: {e}"),
    }
}

async fn exec_deploy(root: Option<PathBuf>, raw: String) {
    let Some(root) = root else {
        error!("No project selected");
        return;
    };
    let ctx = match ProjectContext::load(&root) {
        Ok(ctx) => ctx,
        Err(e) => {
            error!("{e}");
            return;
        }
    };
    let dry_run = raw.split_whitespace().any(|w| w == "--dry-run");
    let staging = raw.split_whitespace().any(|w| w == "--staging");
    if staging {
        match deploy::deploy_staging(&ctx, dry_run).await {
            Ok(msg) => info!("{msg}"),
            Err(e) => error!("Staging deploy failed: {e}"),
        }
    } else {
        match deploy::deploy(&ctx, dry_run).await {
            Ok(msg) => info!("{msg}"),
            Err(e) => error!("Deploy failed: {e}"),
        }
    }
}

async fn exec_rollback(root: Option<PathBuf>, raw: String) {
    let id = raw.split_whitespace().next().unwrap_or("");
    if id.is_empty() {
        error!("No deployment ID provided");
        return;
    }
    let Some(root) = root else {
        error!("No project selected");
        return;
    };
    let ctx = match ProjectContext::load(&root) {
        Ok(ctx) => ctx,
        Err(e) => {
            error!("{e}");
            return;
        }
    };
    match deploy::rollback(&ctx, id).await {
        Ok(msg) => info!("{msg}"),
        Err(e) => error!("Rollback failed: {e}"),
    }
}

async fn exec_clean(root: Option<PathBuf>, _raw: String) {
    let Some(root) = root else {
        error!("No project selected");
        return;
    };
    let ctx = match ProjectContext::load(&root) {
        Ok(ctx) => ctx,
        Err(e) => {
            error!("{e}");
            return;
        }
    };
    match ctx.clean().await {
        Ok(()) => info!("Cleaned build artifacts"),
        Err(e) => error!("Clean failed: {e}"),
    }
}
