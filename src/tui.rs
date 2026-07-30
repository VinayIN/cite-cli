use std::path::{Path, PathBuf};

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
        desc: "Compile project into build/content.json",
        args_hint: "[--force]",
        needs_project: true,
        id: CommandId::Build,
    },
    Cmd {
        label: "doctor",
        desc: "Validate project, metadata, content, and assets",
        args_hint: "",
        needs_project: true,
        id: CommandId::Doctor,
    },
    Cmd {
        label: "deploy",
        desc: "Upload bundle and sync Supabase backend",
        args_hint: "[--dry-run] [--staging]",
        needs_project: true,
        id: CommandId::Deploy,
    },
    Cmd {
        label: "rollback",
        desc: "Roll back to a previous deployment",
        args_hint: "<deployment id>",
        needs_project: true,
        id: CommandId::Rollback,
    },
    Cmd {
        label: "clean",
        desc: "Remove build artifacts and cache",
        args_hint: "",
        needs_project: true,
        id: CommandId::Clean,
    },
];

#[derive(Clone, Copy, PartialEq)]
pub enum TuiMode {
    Runner,
    CommandPalette,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Focus {
    Projects,
    Commands,
    Details,
    Analytics,
    Logs,
}

#[derive(Clone, PartialEq)]
pub enum ProjectItemKind {
    LocalHeader,
    LocalProject(usize),
    ArchivedHeader,
    ArchivedProject(String),
}

#[derive(Clone)]
pub struct ProjectItem {
    pub kind: ProjectItemKind,
    pub label: String,
}

pub struct AnalyticsState {
    pub stats: Option<ProjectStats>,
    pub global: Option<AllStats>,
    pub podcasts: Vec<project::StoredPodcast>,
    pub timelines: Vec<StoredTimeline>,
    pub builds: Vec<StoredBuild>,
    pub deploys: Vec<StoredDeployment>,
    pub stats_expanded: bool,
    pub podcasts_expanded: bool,
    pub timelines_expanded: bool,
    pub builds_expanded: bool,
    pub deploys_expanded: bool,
    pub scroll: usize,
}

pub struct CommandPaletteState {
    pub list_state: ListState,
}

struct EditorPick {
    files: Vec<PathBuf>,
    state: ListState,
    root: PathBuf,
}

pub struct AppState {
    cwd: PathBuf,
    pub roots: Vec<PathBuf>,
    pub db_projects: Vec<(String, String)>,
    pub project_items: Vec<ProjectItem>,
    pub projects_state: ListState,

    pub focus: Focus,
    pub cmds_state: ListState,

    pub log: Vec<String>,
    pub scroll: usize,
    pub busy: bool,
    pub arg_input: String,

    editor_pick: Option<EditorPick>,
    pending_edit: Option<PathBuf>,
    restore_prompt: Option<String>,

    rx: mpsc::Receiver<()>,
    tx: mpsc::Sender<()>,
    task: Option<JoinHandle<()>>,

    mode: TuiMode,
    analytics: AnalyticsState,
    command_palette: CommandPaletteState,

    local_expanded: bool,
    archived_expanded: bool,
}

impl AppState {
    pub async fn new(cwd: &Path) -> Self {
        let (tx, rx) = mpsc::channel(32);
        let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());

        let mut cmds_state = ListState::default();
        cmds_state.select(Some(0));

        let mut state = Self {
            cwd: cwd.clone(),
            roots: Vec::new(),
            db_projects: Vec::new(),
            project_items: Vec::new(),
            projects_state: ListState::default(),
            focus: Focus::Commands,
            cmds_state,
            log: vec![],
            scroll: 0,
            busy: false,
            arg_input: String::new(),
            editor_pick: None,
            pending_edit: None,
            restore_prompt: None,
            rx,
            tx,
            task: None,
            mode: TuiMode::Runner,
            analytics: AnalyticsState {
                stats: None,
                global: None,
                podcasts: Vec::new(),
                timelines: Vec::new(),
                builds: Vec::new(),
                deploys: Vec::new(),
                stats_expanded: true,
                podcasts_expanded: false,
                timelines_expanded: false,
                builds_expanded: false,
                deploys_expanded: false,
                scroll: 0,
            },
            command_palette: CommandPaletteState {
                list_state: ListState::default(),
            },
            local_expanded: true,
            archived_expanded: true,
        };

        state.refresh_projects().await;
        state.load_analytics_data().await;
        state
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
        self.roots.sort();
        self.db_projects = Self::load_db_projects().await;
        self.rebuild_project_items();
    }

    fn rebuild_project_items(&mut self) {
        self.project_items.clear();

        self.project_items.push(ProjectItem {
            kind: ProjectItemKind::LocalHeader,
            label: if self.local_expanded {
                "▼ Local".into()
            } else {
                "▶ Local".into()
            },
        });

        if self.local_expanded {
            if self.roots.is_empty() {
                self.project_items.push(ProjectItem {
                    kind: ProjectItemKind::LocalProject(usize::MAX),
                    label: "  (none)".into(),
                });
            } else {
                for (i, root) in self.roots.iter().enumerate() {
                    let name = root
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("?")
                        .to_string();
                    self.project_items.push(ProjectItem {
                        kind: ProjectItemKind::LocalProject(i),
                        label: format!("  {}", name),
                    });
                }
            }
        }

        self.project_items.push(ProjectItem {
            kind: ProjectItemKind::ArchivedHeader,
            label: if self.archived_expanded {
                "▼ Archived".into()
            } else {
                "▶ Archived".into()
            },
        });

        if self.archived_expanded {
            let archived = self.compute_archived();
            if archived.is_empty() {
                self.project_items.push(ProjectItem {
                    kind: ProjectItemKind::ArchivedProject("".into()),
                    label: "  (none)".into(),
                });
            } else {
                for name in archived {
                    self.project_items.push(ProjectItem {
                        kind: ProjectItemKind::ArchivedProject(name.clone()),
                        label: format!("  {}", name),
                    });
                }
            }
        }
    }

    fn compute_archived(&self) -> Vec<String> {
        self.db_projects
            .iter()
            .filter(|(name, id)| {
                !self.roots.iter().any(|r| {
                    r.to_string_lossy().as_ref() == id.as_str()
                        || r.file_name().and_then(|n| n.to_str()) == Some(name.as_str())
                })
            })
            .map(|(name, _)| name.clone())
            .collect()
    }

    fn selected_root(&self) -> Option<PathBuf> {
        let sel = self.projects_state.selected()?;
        let item = self.project_items.get(sel)?;
        if let ProjectItemKind::LocalProject(i) = item.kind
            && i != usize::MAX
        {
            return self.roots.get(i).cloned();
        }
        None
    }

    fn focus_order(&self) -> Vec<Focus> {
        let sel = self.cmds_state.selected().unwrap_or(0);
        let has_args = !CMDS[sel].args_hint.is_empty();
        let mut order = vec![Focus::Projects, Focus::Commands];
        if has_args {
            order.push(Focus::Details);
        }
        order.push(Focus::Analytics);
        order.push(Focus::Logs);
        order
    }

    fn palette_commands(&self) -> Vec<usize> {
        (0..CMDS.len()).collect()
    }

    pub async fn load_analytics_data(&mut self) {
        let Ok(db) = DbManager::open().await else {
            return;
        };

        self.analytics.global = db.get_all_stats().await.ok();

        if let Some(root) = self.selected_root() {
            let project_id = root.to_string_lossy().to_string();
            self.analytics.stats = db.get_project_stats(&project_id).await.ok();
            self.analytics.podcasts = db
                .get_podcasts_with_content(&project_id)
                .await
                .ok()
                .unwrap_or_default();
            self.analytics.timelines = db.get_timelines(&project_id).await.ok().unwrap_or_default();
            self.analytics.builds = db
                .get_build_history(&project_id)
                .await
                .ok()
                .unwrap_or_default();
            self.analytics.deploys = db
                .get_deployment_history(&project_id)
                .await
                .ok()
                .unwrap_or_default();
        }

        self.analytics.scroll = 0;
    }

    pub async fn handle_key(&mut self, key: KeyEvent) {
        if (key.code == KeyCode::Char('p') || key.code == KeyCode::Char('P'))
            && (key.modifiers.contains(KeyModifiers::SUPER)
                || key.modifiers.contains(KeyModifiers::CONTROL))
        {
            self.mode = match self.mode {
                TuiMode::CommandPalette => TuiMode::Runner,
                TuiMode::Runner => TuiMode::CommandPalette,
            };
            return;
        }

        match self.mode {
            TuiMode::Runner => self.handle_runner_key(key).await,
            TuiMode::CommandPalette => self.handle_command_palette_key(key),
        }
    }

    async fn handle_runner_key(&mut self, key: KeyEvent) {
        if self.restore_prompt.is_some() {
            self.handle_restore_key(key).await;
            return;
        }
        if self.editor_pick.is_some() {
            self.handle_pick_key(key);
            return;
        }

        match key.code {
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if !self.busy {
                    self.analytics.stats = None;
                    self.analytics.global = None;
                    self.analytics.scroll = 0;
                    self.refresh_projects().await;
                    self.load_analytics_data().await;
                    self.log.clear();
                    self.scroll = 0;
                    info!(">> Refreshed");
                }
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::ALT) => {
                if self.focus == Focus::Analytics && !self.busy {
                    self.analytics.podcasts_expanded = !self.analytics.podcasts_expanded;
                }
            }
            KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::ALT) => {
                if self.focus == Focus::Analytics && !self.busy {
                    self.analytics.timelines_expanded = !self.analytics.timelines_expanded;
                }
            }
            KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::ALT) => {
                if self.focus == Focus::Analytics && !self.busy {
                    self.analytics.builds_expanded = !self.analytics.builds_expanded;
                }
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::ALT) => {
                if self.focus == Focus::Analytics && !self.busy {
                    self.analytics.deploys_expanded = !self.analytics.deploys_expanded;
                }
            }
            KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::ALT) => {
                if self.focus == Focus::Projects && !self.busy {
                    self.local_expanded = !self.local_expanded;
                    self.rebuild_project_items();
                }
            }
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::ALT) => {
                if self.focus == Focus::Projects && !self.busy {
                    self.archived_expanded = !self.archived_expanded;
                    self.rebuild_project_items();
                }
            }
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
                Focus::Analytics => {
                    self.analytics.scroll = self.analytics.scroll.saturating_sub(1);
                }
                Focus::Logs => self.scroll = self.scroll.saturating_sub(1),
                _ => {}
            },
            KeyCode::Down => match self.focus {
                Focus::Projects if !self.busy => self.projects_state.select_next(),
                Focus::Analytics => {
                    self.analytics.scroll = self.analytics.scroll.saturating_add(1);
                }
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
                            let Some(item) = self.project_items.get(sel) else {
                                return;
                            };
                            match &item.kind {
                                ProjectItemKind::LocalHeader => {
                                    self.local_expanded = !self.local_expanded;
                                    self.rebuild_project_items();
                                }
                                ProjectItemKind::ArchivedHeader => {
                                    self.archived_expanded = !self.archived_expanded;
                                    self.rebuild_project_items();
                                }
                                ProjectItemKind::LocalProject(_) => {
                                    info!(">> Selected project");
                                    self.load_analytics_data().await;
                                    self.open_edit_picker();
                                }
                                ProjectItemKind::ArchivedProject(name) => {
                                    self.restore_prompt = Some(name.clone());
                                }
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
            KeyCode::PageUp if matches!(self.focus, Focus::Analytics) => {
                self.analytics.scroll = self.analytics.scroll.saturating_sub(10)
            }
            KeyCode::PageDown if matches!(self.focus, Focus::Analytics) => {
                self.analytics.scroll = self.analytics.scroll.saturating_add(10)
            }
            _ => {}
        }
    }

    pub fn start_cmd(&mut self) {
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
                CommandId::Doctor => exec_doctor(root, raw_args).await,
                CommandId::Deploy => exec_deploy(root, raw_args).await,
                CommandId::Rollback => exec_rollback(root, raw_args).await,
                CommandId::Clean => exec_clean(root, raw_args).await,
            }
            let _ = tx.send(()).await;
        });
        self.task = Some(handle);
    }

    fn handle_command_palette_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.mode = TuiMode::Runner,
            KeyCode::Up => self.command_palette.list_state.select_previous(),
            KeyCode::Down => self.command_palette.list_state.select_next(),
            KeyCode::Enter => {
                let all = self.palette_commands();
                if let Some(&cmd_idx) = self
                    .command_palette
                    .list_state
                    .selected()
                    .and_then(|i| all.get(i))
                {
                    self.cmds_state.select(Some(cmd_idx));
                    self.mode = TuiMode::Runner;
                    self.focus = Focus::Commands;
                    self.start_cmd();
                }
            }
            KeyCode::Char(c) => {
                self.focus = Focus::Details;
                self.arg_input.push(c);
                self.mode = TuiMode::Runner;
            }
            KeyCode::Backspace => {
                if !self.arg_input.is_empty() {
                    self.arg_input.pop();
                }
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

    async fn handle_restore_key(&mut self, key: KeyEvent) {
        let Some(name) = self.restore_prompt.clone() else {
            return;
        };
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                let target = self.cwd.join(&name);
                if let Err(e) = scaffold::init_project(&name, &target) {
                    error!("Failed to restore project: {e}");
                } else {
                    info!("Restored project {name} at {}", target.display());
                    self.refresh_projects().await;
                    self.load_analytics_data().await;
                    if let Some(idx) = self
                        .project_items
                        .iter()
                        .position(|item| matches!(&item.kind, ProjectItemKind::LocalProject(_)))
                    {
                        self.projects_state.select(Some(idx));
                        self.open_edit_picker();
                    }
                }
                self.restore_prompt = None;
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Char('q') => {
                self.restore_prompt = None;
            }
            _ => {}
        }
    }

    fn open_edit_picker(&mut self) {
        let Some(root) = self.selected_root() else {
            error!("No project selected");
            return;
        };
        let mut files = Vec::new();
        collect_files(&root, &root, &mut files);
        files.sort();
        let mut state = ListState::default();
        state.select(Some(0));
        self.editor_pick = Some(EditorPick { files, state, root });
    }
}

fn collect_files(base: &Path, dir: &Path, files: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name == "build" || name.starts_with('.') {
                        continue;
                    }
                }
                collect_files(base, &path, files);
            } else if path.is_file() {
                files.push(path);
            }
        }
    }
}

fn block(title: impl Into<String>, focused: bool) -> Block<'static> {
    let border_style = if focused {
        Style::new()
    } else {
        Style::new().dim()
    };
    Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
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
                    if event::poll(std::time::Duration::from_millis(100)).unwrap_or(false) {
                        event::read().ok()
                    } else {
                        None
                    }
                }) => {
                    if let Ok(Some(event)) = result {
                        if event_tx.send(event).is_err() {
                            break;
                        }
                    }
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
            Some(()) = app.rx.recv() => {
                app.busy = false;
                app.task = None;
                info!(">> Command complete, refreshing");
                app.refresh_projects().await;
                app.load_analytics_data().await;
            }
            Some(line) = log_rx.recv() => {
                let was_at_bottom = app.scroll >= app.log.len().saturating_sub(1);
                app.log.push(line);
                if was_at_bottom || app.log.len() <= 1 {
                    app.scroll = app.log.len().saturating_sub(1);
                }
            }
            Some(event) = event_rx.recv() => {
                if let Event::Key(key) = event {
                    if key.kind == KeyEventKind::Press {
                        if (key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL))
                            || (key.code == KeyCode::Char('q') && key.modifiers.is_empty() && app.mode == TuiMode::Runner)
                            || (key.code == KeyCode::Esc && app.mode == TuiMode::Runner && app.editor_pick.is_none())
                        {
                            break;
                        }
                        app.handle_key(key).await;

                        if let Some(path) = app.pending_edit.take() {
                            edit_file(&mut terminal, &mut app, &path)
                                .await
                                .map_err(|e| CiteError::Config(format!("{e}")))?;
                        }
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

    if app.restore_prompt.is_some() {
        render_restore_prompt(frame, frame.area(), app);
    } else if app.editor_pick.is_some() {
        render_editor_pick(frame, frame.area(), app);
    }

    if app.mode == TuiMode::CommandPalette {
        render_command_palette(frame, frame.area(), app);
    }
}

fn render_header(frame: &mut Frame, area: Rect, app: &AppState) {
    let (left_text, style) = if app.busy {
        let cmd = &CMDS[app.cmds_state.selected().unwrap_or(0)];
        (
            format!(" Executing: {} on {} ", cmd.label, app.cwd.display()),
            Style::new()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        (
            " Ready ".to_string(),
            Style::new().fg(Color::Black).bg(Color::Green),
        )
    };
    let version = Span::styled(
        format!("v{}", env!("CARGO_PKG_VERSION")),
        Style::new().dim(),
    );

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
            let [left, middle, right] = Layout::horizontal([
                Constraint::Max(20),
                Constraint::Fill(2),
                Constraint::Fill(1),
            ])
            .areas(area);

            render_categorized_project_list(frame, left, app);

            let [tabs_area, details_logs_area] =
                Layout::vertical([Constraint::Max(3), Constraint::Fill(1)]).areas(middle);

            let [details_area, logs_area] =
                Layout::vertical([Constraint::Max(6), Constraint::Fill(1)])
                    .areas(details_logs_area);

            render_cmd_tabs(frame, tabs_area, app);
            render_cmd_doc(frame, details_area, app);
            render_log(frame, logs_area, app);

            render_analytics_content(frame, right, app);
        }
    }
}

fn render_categorized_project_list(frame: &mut Frame, area: Rect, app: &mut AppState) {
    let is_focused = matches!(app.focus, Focus::Projects);

    let items: Vec<ListItem> = app
        .project_items
        .iter()
        .map(|item| {
            let style = match item.kind {
                ProjectItemKind::LocalHeader | ProjectItemKind::ArchivedHeader => {
                    if is_focused {
                        Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                    } else {
                        Style::new().add_modifier(Modifier::BOLD)
                    }
                }
                _ => Style::new(),
            };
            ListItem::new(Line::from(Span::styled(item.label.clone(), style)))
        })
        .collect();

    if let Some(sel) = app.projects_state.selected() {
        if sel >= items.len() {
            app.projects_state
                .select(Some(items.len().saturating_sub(1)));
        }
    } else if !items.is_empty() {
        app.projects_state.select(Some(0));
    }

    let block_widget = block(" Projects ", is_focused);
    let inner_area = block_widget.inner(area);
    frame.render_widget(block_widget, area);

    let list = List::new(items).highlight_style(
        Style::new()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );
    frame.render_stateful_widget(list, inner_area, &mut app.projects_state);

    let total = app.project_items.len();
    let visible = inner_area.height as usize;
    if total > visible {
        let selected = app.projects_state.selected().unwrap_or(0);
        let max_scroll = total.saturating_sub(visible);
        let scroll_pos = selected.min(max_scroll);
        let mut scroll_state = ScrollbarState::default()
            .content_length(total)
            .position(scroll_pos);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None),
            area,
            &mut scroll_state,
        );
    }
}

fn render_cmd_tabs(frame: &mut Frame, area: Rect, app: &mut AppState) {
    let is_focused = matches!(app.focus, Focus::Commands);
    let titles: Vec<Line> = CMDS.iter().map(|cmd| Line::from(cmd.label)).collect();
    let tabs = Tabs::new(titles)
        .block(block(" Commands ", is_focused))
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

    let mut lines = vec![Line::from(vec![
        Span::styled(format!("{}: ", cmd.label), Style::new().bold()),
        Span::raw(cmd.desc),
    ])];

    if has_args {
        lines.extend_from_slice(&[Line::from(format!("Arguments: {}", cmd.args_hint))]);
        let input_text = if app.arg_input.is_empty() {
            "Awaiting input...".to_string()
        } else {
            format!("{}", app.arg_input)
        };
        let cursor_style = if is_focused {
            Style::new()
                .fg(Color::Yellow)
                .add_modifier(Modifier::UNDERLINED)
        } else {
            Style::new()
        };
        lines.push(Line::from(vec![
            Span::raw("Input: "),
            Span::styled(input_text, cursor_style),
        ]));
    }

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block(" Details ", is_focused))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_log(frame: &mut Frame, area: Rect, app: &AppState) {
    let is_focused = matches!(app.focus, Focus::Logs);
    let visible_lines = area.height.saturating_sub(2) as usize;
    let max_scroll = app.log.len().saturating_sub(visible_lines);
    let scroll_y = app.scroll.min(max_scroll);
    let end = (scroll_y + visible_lines).min(app.log.len());

    let lines: Vec<Line> = app
        .log
        .get(scroll_y..end)
        .unwrap_or(&[])
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

fn pane_label(focus: Focus) -> &'static str {
    match focus {
        Focus::Projects => " Projects ",
        Focus::Commands => " Commands ",
        Focus::Details => " Details ",
        Focus::Analytics => " Analytics ",
        Focus::Logs => " Logs ",
    }
}

fn render_statusbar(frame: &mut Frame, area: Rect, app: &AppState) {
    let mode_label = match app.mode {
        TuiMode::Runner => pane_label(app.focus),
        TuiMode::CommandPalette => " Command Palette ",
    };

    let help_text = match app.mode {
        TuiMode::CommandPalette => "[↑/↓] [Enter] [Esc]",
        TuiMode::Runner => match app.focus {
            Focus::Projects => "[↑/↓] [Ctrl+R] [Enter] [Alt+L/A]",
            Focus::Commands => "[←/→] [Enter]",
            Focus::Details => "[Enter]",
            Focus::Analytics => "[↑/↓] [Ctrl+R] [Enter] [Alt+P/T/B/D]",
            Focus::Logs => "[↑/↓] [PgUp/PgDn]",
        },
    };

    let [left_area, right_area] =
        Layout::horizontal([Constraint::Length(18), Constraint::Fill(1)]).areas(area);

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
    let all = app.palette_commands();

    let items: Vec<ListItem> = all
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
        .title(" Command Palette ")
        .border_style(Style::new().fg(Color::Yellow));

    let inner = palette_block.inner(popup);
    frame.render_widget(palette_block, popup);

    let [list_area] = Layout::vertical([Constraint::Fill(1)]).areas(inner);
    frame.render_stateful_widget(list, list_area, &mut app.command_palette.list_state);
}

fn render_restore_prompt(frame: &mut Frame, area: Rect, app: &AppState) {
    let Some(ref name) = app.restore_prompt else {
        return;
    };
    let width = 50u16.min(area.width.saturating_sub(2));
    let height = 6u16;
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

    let text = Text::from(vec![
        Line::from(vec![Span::raw(format!(
            "Restore project \"{}\" Locally?",
            name
        ))]),
        Line::from(""),
        Line::from(Span::styled("[Y]es  [N]o", Style::new().bold())),
    ]);
    let p = Paragraph::new(text)
        .block(block(" Restore ", true))
        .alignment(Alignment::Center);
    frame.render_widget(Clear, popup);
    frame.render_widget(p, popup);
}

fn render_editor_pick(frame: &mut Frame, area: Rect, app: &mut AppState) {
    let Some(pick) = &mut app.editor_pick else {
        return;
    };
    let root = &pick.root;
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
        .map(|f| ListItem::new(f.strip_prefix(&root).unwrap_or(f).to_string_lossy()))
        .collect();
    let list = List::new(items)
        .block(block(" Select File to Edit ", true))
        .highlight_style(Style::new().bold())
        .highlight_symbol("▸ ");

    frame.render_widget(Clear, popup);
    frame.render_stateful_widget(list, popup, &mut pick.state);
}

fn render_analytics_content(frame: &mut Frame, area: Rect, app: &AppState) {
    let mut lines: Vec<Line> = Vec::new();
    render_analytics_global(&mut lines, &app.analytics);
    render_analytics_project_stats(&mut lines, &app.analytics);
    render_analytics_podcasts(&mut lines, &app.analytics);
    render_analytics_timelines(&mut lines, &app.analytics);
    render_analytics_builds(&mut lines, &app.analytics);
    render_analytics_deployments(&mut lines, &app.analytics);

    let is_focused = matches!(app.focus, Focus::Analytics);
    let block_widget = block(" Analytics ", is_focused);
    let inner_area = block_widget.inner(area);
    frame.render_widget(block_widget, area);

    let visible_lines = inner_area.height as usize;
    let total_lines = lines.len();
    let max_scroll = total_lines.saturating_sub(visible_lines);
    let scroll_y = app.analytics.scroll.min(max_scroll);

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: false })
            .scroll((scroll_y as u16, 0)),
        inner_area,
    );

    if total_lines > visible_lines {
        let mut scroll_state = ScrollbarState::default()
            .content_length(total_lines)
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

fn render_analytics_global(lines: &mut Vec<Line>, analytics: &AnalyticsState) {
    lines.push(Line::from(Span::styled(
        "Global Summary",
        Style::new().bold(),
    )));
    if let Some(ref global) = analytics.global {
        lines.push(Line::from(format!(
            "  Projects   : {}",
            global.project_count
        )));
        lines.push(Line::from(format!(
            "  Podcasts   : {}",
            global.total_podcasts
        )));
        lines.push(Line::from(format!(
            "  Timelines  : {}",
            global.total_timelines
        )));
        lines.push(Line::from(format!("  Words      : {}", global.total_words)));
        lines.push(Line::from(format!(
            "  Builds     : {}",
            global.total_builds
        )));
    } else {
        lines.push(Line::from("  No data"));
    }
    lines.push(Line::from(""));
}

fn render_analytics_project_stats(lines: &mut Vec<Line>, analytics: &AnalyticsState) {
    let label = if analytics.stats_expanded {
        "▼"
    } else {
        "▶"
    };
    lines.push(Line::from(Span::styled(
        "Project Statistics",
        Style::new().bold(),
    )));
    lines.push(Line::from(label));
    if analytics.stats_expanded {
        if let Some(ref stats) = analytics.stats {
            lines.push(Line::from(format!(
                "  Podcasts     : {}",
                stats.podcast_count
            )));
            lines.push(Line::from(format!(
                "  Total Words  : {}",
                stats.total_words
            )));
            lines.push(Line::from(format!(
                "  Timelines    : {}",
                stats.timeline_count
            )));
            lines.push(Line::from(format!(
                "  Builds       : {}",
                stats.build_count
            )));
            lines.push(Line::from(format!(
                "  Deployments  : {}",
                stats.deployment_count
            )));
            if let Some(ref last) = stats.last_built {
                lines.push(Line::from(format!(
                    "  Last Build   : {}",
                    last.get(..19).unwrap_or(last)
                )));
            }
            if let Some(ref last) = stats.last_deployed {
                lines.push(Line::from(format!(
                    "  Last Deploy  : {}",
                    last.get(..19).unwrap_or(last)
                )));
            }
        }
    }
    lines.push(Line::from(""));
}

fn render_analytics_podcasts(lines: &mut Vec<Line>, analytics: &AnalyticsState) {
    let label = if analytics.podcasts_expanded {
        "▼"
    } else {
        "▶"
    };
    lines.push(Line::from(Span::styled(
        "Podcasts Metadata",
        Style::new().bold(),
    )));
    lines.push(Line::from(label));
    if analytics.podcasts_expanded {
        for p in &analytics.podcasts {
            let tag = if p.category.is_empty() {
                String::new()
            } else {
                format!(" [{}]", p.category)
            };
            let audio_flag = if p.has_audio { " [A]" } else { "" };
            let thumb_flag = if p.has_thumbnail { " [T]" } else { "" };
            let file_name = p.file.rsplit('/').next().unwrap_or(&p.file);
            lines.push(Line::from(format!(
                "  {}{}{}{}  ({}w) <{}>",
                p.title, tag, audio_flag, thumb_flag, p.word_count, file_name
            )));
        }
    }
    lines.push(Line::from(""));
}

fn render_analytics_timelines(lines: &mut Vec<Line>, analytics: &AnalyticsState) {
    let label = if analytics.timelines_expanded {
        "▼"
    } else {
        "▶"
    };
    lines.push(Line::from(Span::styled("Timelines", Style::new().bold())));
    lines.push(Line::from(label));
    if analytics.timelines_expanded {
        for t in &analytics.timelines {
            let mut extra = String::new();
            if let Some(et) = &t.entry_type {
                extra = format!(" ({})", et);
            }
            if t.url.is_some() {
                extra.push_str(" link");
            }
            lines.push(Line::from(format!(
                "  {}  {}{}",
                t.date.as_deref().unwrap_or("??"),
                t.title,
                extra
            )));
        }
    }
    lines.push(Line::from(""));
}

fn render_analytics_builds(lines: &mut Vec<Line>, analytics: &AnalyticsState) {
    let label = if analytics.builds_expanded {
        "▼"
    } else {
        "▶"
    };
    lines.push(Line::from(Span::styled(
        "Build History",
        Style::new().bold(),
    )));
    lines.push(Line::from(label));
    if analytics.builds_expanded {
        for b in &analytics.builds {
            let date = b.built_at.get(..16).unwrap_or(&b.built_at);
            let flag = if b.was_incremental {
                "(incr)"
            } else {
                "(full)"
            };
            lines.push(Line::from(format!(
                "  {} p:{} w:{} {:>4}ms {}",
                date, b.podcast_count, b.total_words, b.duration_ms, flag
            )));
        }
    }
    lines.push(Line::from(""));
}

fn render_analytics_deployments(lines: &mut Vec<Line>, analytics: &AnalyticsState) {
    let label = if analytics.deploys_expanded {
        "▼"
    } else {
        "▶"
    };
    lines.push(Line::from(Span::styled(
        "Deployment History",
        Style::new().bold(),
    )));
    lines.push(Line::from(label));
    if analytics.deploys_expanded {
        for d in &analytics.deploys {
            let status = if d.success { "ok" } else { "fail" };
            lines.push(Line::from(format!(
                "  {}  {}  {}  n:{} a:{}",
                d.deployment_id, d.deployed_at, status, d.news_count, d.asset_count
            )));
        }
    }
}

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

async fn exec_init(cwd: PathBuf, raw: String) {
    let name = raw.split_whitespace().next().unwrap_or("new-project");
    let target = cwd.join(name);
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
    let Ok(ctx) = ProjectContext::load(&root) else {
        error!("Failed to load project");
        return;
    };
    let Ok(db) = DbManager::open().await else {
        error!("Failed to open database");
        return;
    };
    let force = raw.split_whitespace().any(|w| w == "--force");
    match compiler::compile(&db, &ctx, force).await {
        Ok(outcome) => outcome.emit(),
        Err(e) => error!("Build failed: {e}"),
    }
}

async fn exec_doctor(root: Option<PathBuf>, _raw: String) {
    let Some(root) = root else {
        error!("No project selected");
        return;
    };
    let Ok(ctx) = ProjectContext::load(&root) else {
        error!("Failed to load project");
        return;
    };
    let Ok(db) = DbManager::open().await else {
        error!("Failed to open database");
        return;
    };
    match doctor::run(&db, &ctx).await {
        Ok(o) => {
            o.emit();
            project::print_status(&db, &ctx).await;
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
    let Ok(ctx) = ProjectContext::load(&root) else {
        error!("Failed to load project");
        return;
    };
    let Ok(db) = DbManager::open().await else {
        error!("Failed to open database");
        return;
    };
    let dry_run = raw.split_whitespace().any(|w| w == "--dry-run");
    let staging = raw.split_whitespace().any(|w| w == "--staging");
    if staging {
        match deploy::deploy_staging(&db, &ctx, dry_run).await {
            Ok(msg) => info!("{msg}"),
            Err(e) => error!("Staging deploy failed: {e}"),
        }
    } else {
        match deploy::deploy(&db, &ctx, dry_run).await {
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
    let Ok(ctx) = ProjectContext::load(&root) else {
        error!("Failed to load project");
        return;
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
    let Ok(ctx) = ProjectContext::load(&root) else {
        error!("Failed to load project");
        return;
    };
    let Ok(db) = DbManager::open().await else {
        error!("Failed to open database");
        return;
    };
    match ctx.clean(&db).await {
        Ok(()) => info!("Cleaned build artifacts"),
        Err(e) => error!("Clean failed: {e}"),
    }
}
