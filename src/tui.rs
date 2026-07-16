use std::path::{Path, PathBuf};
use std::time::Duration;

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::core::CiteError;
use crate::core::project::{self, ProjectContext};
use crate::core::{compiler, deploy, doctor, scaffold};

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

// Main entry point for the TUI
pub async fn run_tui(mut log_rx: mpsc::UnboundedReceiver<String>) -> Result<(), CiteError> {
    let mut terminal = ratatui::init();
    let _guard = TerminalGuard;

    terminal
        .clear()
        .map_err(|e| CiteError::Config(format!("{e}")))?;

    let cwd = std::env::current_dir().unwrap_or_default();
    let mut app = AppState::new(&cwd);

    loop {
        terminal
            .draw(|f| render(f, &mut app))
            .map_err(|e| CiteError::Config(format!("{e}")))?;

        tokio::select! {
            biased;
            Some(()) = app.rx.recv() => {
                app.busy = false;
                app.task = None;
                app.refresh_projects();
            }
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                if app.busy && app.task.as_ref().map_or(false, |h| h.is_finished()) {
                    app.busy = false;
                    app.task = None;
                }

                while let Ok(line) = log_rx.try_recv() {
                    app.log.push(line);
                    app.scroll = app.log.len().saturating_sub(1);
                }

                if event::poll(Duration::from_millis(10))
                    .map_err(|e| CiteError::Config(format!("{e}")))?
                {
                    match event::read().map_err(|e| CiteError::Config(format!("{e}")))? {
                        Event::Key(key) if key.kind == KeyEventKind::Press => {
                            if key.code == KeyCode::Esc && app.editor_pick.is_none() {
                                break;
                            }
                            app.handle_key(key);

                            if let Some(path) = app.pending_edit.take() {
                                edit_file(&mut terminal, &mut app, &path)
                                    .map_err(|e| CiteError::Config(format!("{e}")))?;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    Ok(())
}

// Suspend the TUI, open `path` in the user's editor, then restore the TUI.
fn edit_file(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut AppState,
    path: &Path,
) -> std::io::Result<()> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());

    info!(">> Spelunking {} in {editor}", path.display());

    let before = file_digest(path);

    ratatui::restore();
    let status = std::process::Command::new(&editor).arg(path).status();
    *terminal = ratatui::init();
    terminal.clear()?;

    match status {
        Ok(s) if s.success() => {
            if file_digest(path) != before {
                info!("Edited {}", path.display());
            } else {
                info!("No changes to {}", path.display());
            }
        }
        Ok(s) => warn!("Editor exited with {s}"),
        Err(e) => error!("Failed to launch '{editor}': {e}"),
    }
    app.refresh_projects();
    Ok(())
}

fn file_digest(path: &Path) -> Option<[u8; 32]> {
    use sha2::Digest;
    let bytes = std::fs::read(path).ok()?;
    Some(sha2::Sha256::digest(&bytes).into())
}

// Data Types

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
        desc: "Deploy the built project to Supabase staging",
        args_hint: "[--dry-run]",
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
pub enum Focus {
    Projects,
    Commands,
    Details,
    Logs,
}

impl Focus {
    fn label(self) -> &'static str {
        match self {
            Focus::Projects => "Projects",
            Focus::Commands => "Commands",
            Focus::Details => "Details",
            Focus::Logs => "Logs",
        }
    }
}

struct EditorPick {
    files: Vec<PathBuf>,
    sel: usize,
}

pub struct AppState {
    cwd: PathBuf,
    pub roots: Vec<PathBuf>,
    pub sel_project: usize,
    pub focus: Focus,
    pub sel_cmd: usize,
    pub log: Vec<String>,
    pub scroll: usize,
    pub busy: bool,
    pub arg_input: String,
    editor_pick: Option<EditorPick>,
    pending_edit: Option<PathBuf>,
    rx: mpsc::Receiver<()>,
    tx: mpsc::Sender<()>,
    task: Option<JoinHandle<()>>,
}

impl AppState {
    pub fn new(cwd: &Path) -> Self {
        let (tx, rx) = mpsc::channel(32);
        let roots = Self::discover(cwd);

        Self {
            cwd: cwd.to_path_buf(),
            roots,
            sel_project: 0,
            focus: Focus::Commands,
            sel_cmd: 0,
            log: vec![],
            scroll: 0,
            busy: false,
            arg_input: String::new(),
            editor_pick: None,
            pending_edit: None,
            rx,
            tx,
            task: None,
        }
    }

    fn discover(cwd: &Path) -> Vec<PathBuf> {
        let mut r = project::discover_projects(cwd);
        r.sort();
        r
    }

    fn refresh_projects(&mut self) {
        self.cwd = std::env::current_dir().unwrap_or_else(|_| self.cwd.clone());
        self.roots = Self::discover(&self.cwd);
        self.sel_project = self.sel_project.min(self.roots.len().saturating_sub(1));
    }

    fn selected_root(&self) -> Option<PathBuf> {
        self.roots.get(self.sel_project).cloned()
    }

    fn focus_order(&self) -> Vec<Focus> {
        let mut order = vec![Focus::Projects, Focus::Commands];
        if !CMDS[self.sel_cmd].args_hint.is_empty() {
            order.push(Focus::Details);
        }
        order.push(Focus::Logs);
        order
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if self.editor_pick.is_some() {
            self.handle_pick_key(key);
            return;
        }

        if self.busy {
            return;
        }

        let has_args = !CMDS[self.sel_cmd].args_hint.is_empty();

        match key.code {
            KeyCode::Tab => {
                let order = self.focus_order();
                let i = order.iter().position(|f| *f == self.focus).unwrap_or(0);
                self.focus = order[(i + 1) % order.len()];
            }
            KeyCode::BackTab => {
                let order = self.focus_order();
                let i = order.iter().position(|f| *f == self.focus).unwrap_or(0);
                let n = order.len();
                self.focus = order[(i + n - 1) % n];
            }
            KeyCode::Up => match self.focus {
                Focus::Projects => self.sel_project = self.sel_project.saturating_sub(1),
                Focus::Commands | Focus::Details => {
                    self.sel_cmd = self.sel_cmd.saturating_sub(1);
                    if !has_args {
                        self.focus = Focus::Commands;
                    }
                }
                Focus::Logs => self.scroll = self.scroll.saturating_sub(1),
            },
            KeyCode::Down => match self.focus {
                Focus::Projects => {
                    self.sel_project =
                        (self.sel_project + 1).min(self.roots.len().saturating_sub(1));
                }
                Focus::Commands | Focus::Details => {
                    self.sel_cmd = (self.sel_cmd + 1).min(CMDS.len().saturating_sub(1));
                    if !has_args {
                        self.focus = Focus::Commands;
                    }
                }
                Focus::Logs => self.scroll = self.scroll.saturating_add(1),
            },
            KeyCode::Enter => match self.focus {
                Focus::Projects => self.open_edit_picker(),
                Focus::Commands | Focus::Details => self.start_cmd(),
                _ => {}
            },
            KeyCode::Backspace => {
                if matches!(self.focus, Focus::Details) {
                    self.arg_input.pop();
                }
            }
            KeyCode::Char('r') if !matches!(self.focus, Focus::Details) => {
                self.refresh_projects();
                self.log.clear();
                self.scroll = 0;
                warn!(">> Refreshed");
            }
            KeyCode::Char(ch) => {
                if matches!(self.focus, Focus::Details) {
                    self.arg_input.push(ch);
                }
            }
            KeyCode::PageUp => {
                if matches!(self.focus, Focus::Logs) {
                    self.scroll = self.scroll.saturating_sub(10);
                }
            }
            KeyCode::PageDown => {
                if matches!(self.focus, Focus::Logs) {
                    self.scroll = self.scroll.saturating_add(10);
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
            KeyCode::Up => pick.sel = pick.sel.saturating_sub(1),
            KeyCode::Down => pick.sel = (pick.sel + 1).min(pick.files.len().saturating_sub(1)),
            KeyCode::Enter => {
                self.pending_edit = pick.files.get(pick.sel).cloned();
                self.editor_pick = None;
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
        self.editor_pick = Some(EditorPick { files, sel: 0 });
    }

    fn start_cmd(&mut self) {
        let root = self.selected_root();
        let cmd = &CMDS[self.sel_cmd];

        if cmd.needs_project && root.is_none() {
            error!("No projects found — select or init a project first");
            return;
        }

        let raw_args = std::mem::take(&mut self.arg_input);
        let arg_display = if raw_args.is_empty() {
            String::new()
        } else {
            format!(" ({raw_args})")
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

        let handle = tokio::spawn(async move {
            match id {
                CommandId::Init => exec_init(root, raw_args).await,
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

// Rendering

fn block(title: &str, focused: bool) -> Block<'_> {
    let border_style = if focused {
        Style::new().fg(Color::Cyan).bold()
    } else {
        Style::new()
    };
    Block::default()
        .borders(Borders::ALL)
        .title(format!(" {title} "))
        .border_style(border_style)
}

fn render(frame: &mut Frame, app: &mut AppState) {
    let [header, body, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    render_header(frame, header);
    render_body(frame, body, app);
    render_statusbar(frame, status, app);

    if let Some(pick) = &app.editor_pick {
        render_editor_pick(frame, frame.area(), pick);
    }
}

fn render_editor_pick(frame: &mut Frame, area: Rect, pick: &EditorPick) {
    let width = 54u16.min(area.width.saturating_sub(2));
    let height = (pick.files.len() as u16 + 2).min(area.height.saturating_sub(2));
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

    let inner_width = popup.width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = pick
        .files
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let is_selected = i == pick.sel;
            let prefix = if is_selected { "▸ " } else { "  " };
            let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            let text = format!("{prefix}{name}");
            let padded = format!("{text:<width$}", width = inner_width);
            let style = if is_selected {
                Style::new().bold().bg(Color::Cyan).fg(Color::Black)
            } else {
                Style::new()
            };
            ListItem::new(Line::from(Span::styled(padded, style))).style(style)
        })
        .collect();

    frame.render_widget(Clear, popup);
    frame.render_widget(List::new(items).block(block("Edit", true)), popup);
}

fn render_header(frame: &mut Frame, area: Rect) {
    let t = Line::from(vec![Span::styled(
        format!("v{}", env!("CARGO_PKG_VERSION")),
        Style::new().fg(Color::DarkGray),
    )]);
    frame.render_widget(Paragraph::new(t).alignment(Alignment::Right), area);
}

fn render_body(frame: &mut Frame, area: Rect, app: &AppState) {
    let [left, right] = Layout::horizontal([Constraint::Max(25), Constraint::Fill(1)]).areas(area);
    let [top, bot] = Layout::vertical([Constraint::Max(12), Constraint::Fill(1)]).areas(right);
    let [cmd_list, cmd_doc] =
        Layout::horizontal([Constraint::Max(25), Constraint::Fill(1)]).areas(top);

    render_projects(frame, left, app);
    render_cmds(frame, cmd_list, app);
    render_cmd_doc(frame, cmd_doc, app);
    render_log(frame, bot, app);
}

fn render_projects(frame: &mut Frame, area: Rect, app: &AppState) {
    let is_focused = matches!(app.focus, Focus::Projects);
    let inner_width = area.width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = app
        .roots
        .iter()
        .enumerate()
        .map(|(i, root)| {
            let is_selected = i == app.sel_project;
            let prefix = if is_selected { "▸ " } else { "  " };
            let name = root.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            let text = format!("{prefix}{name}");
            let padded = format!("{text:<width$}", width = inner_width);
            let style = if is_focused && is_selected {
                Style::new().bold().bg(Color::Cyan).fg(Color::Black)
            } else if is_selected {
                Style::new().bold().fg(Color::Cyan)
            } else {
                Style::new()
            };
            ListItem::new(Line::from(Span::styled(padded, style))).style(style)
        })
        .collect();

    let title = format!("Projects ({})", app.roots.len());
    frame.render_widget(List::new(items).block(block(&title, is_focused)), area);
}

fn render_cmds(frame: &mut Frame, area: Rect, app: &AppState) {
    let is_focused = matches!(app.focus, Focus::Commands);
    let inner_width = area.width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = CMDS
        .iter()
        .enumerate()
        .map(|(i, cmd)| {
            let is_selected = i == app.sel_cmd;
            let prefix = if is_selected { "▸ " } else { "  " };
            let text = format!("{prefix}{}", cmd.label);
            let padded = format!("{text:<width$}", width = inner_width);
            let style = if is_focused && is_selected {
                Style::new().bold().bg(Color::Cyan).fg(Color::Black)
            } else if is_selected {
                Style::new().bold().fg(Color::Cyan)
            } else {
                Style::new()
            };
            ListItem::new(Line::from(Span::styled(padded, style))).style(style)
        })
        .collect();

    frame.render_widget(List::new(items).block(block("Commands", is_focused)), area);
}

fn render_cmd_doc(frame: &mut Frame, area: Rect, app: &AppState) {
    let cmd = &CMDS[app.sel_cmd];
    let is_focused = matches!(app.focus, Focus::Details);
    let has_args = !cmd.args_hint.is_empty();

    let mut lines = vec![
        Line::from(Span::styled(cmd.label, Style::new().bold())),
        Line::from(""),
        Line::from(cmd.desc),
    ];

    if has_args {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::raw("Arguments:")));
        lines.push(Line::from(Span::raw(format!("  {}", cmd.args_hint))));

        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::raw("Input: "),
            Span::styled(
                if app.arg_input.is_empty() {
                    "Awaiting input..."
                } else {
                    app.arg_input.as_str()
                },
                if is_focused {
                    Style::new()
                        .bold()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::UNDERLINED)
                } else {
                    Style::new().fg(Color::Gray)
                },
            ),
        ]));
    }

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(block("Details", is_focused))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_log(frame: &mut Frame, area: Rect, app: &AppState) {
    let visible_lines = area.height.saturating_sub(3) as usize;
    let max_scroll = app.log.len().saturating_sub(visible_lines);
    let scroll_y = app.scroll.min(max_scroll);

    let end = (scroll_y + visible_lines).min(app.log.len());
    let lines: Vec<Line> = app.log[scroll_y..end]
        .iter()
        .map(|l| color_log_line(l.as_str()))
        .collect();

    let para = Paragraph::new(Text::from(lines))
        .block(block("Logs", matches!(app.focus, Focus::Logs)))
        .wrap(Wrap { trim: false });

    frame.render_widget(para, area);
}

fn color_log_line(l: &str) -> Line<'static> {
    const TAGS: &[(&str, Color)] = &[
        ("ERROR", Color::Red),
        ("WARN", Color::Yellow),
        ("INFO", Color::Green),
    ];

    let mut spans = Vec::new();
    let mut remaining = l;

    for (tag, color) in TAGS {
        if let Some(idx) = remaining.find(tag) {
            spans.push(Span::raw(remaining[..idx].to_string()));
            spans.push(Span::styled(*tag, Style::new().fg(*color).bold()));
            remaining = &remaining[idx + tag.len()..];
        }
    }
    spans.push(Span::raw(remaining.to_string()));
    Line::from(spans)
}

fn render_statusbar(frame: &mut Frame, area: Rect, app: &AppState) {
    let mut left_spans = vec![];

    if app.busy {
        left_spans.push(Span::styled(
            " RUNNING ",
            Style::new().bg(Color::Yellow).fg(Color::Black),
        ));
    } else {
        left_spans.push(Span::styled(
            " READY ",
            Style::new().bg(Color::Green).fg(Color::Black),
        ));
    }

    left_spans.push(Span::styled(
        format!("Panel: {} ", app.focus.label()),
        Style::new().bold().bg(Color::Cyan).fg(Color::Black),
    ));

    if let Some(proj_name) = app
        .selected_root()
        .and_then(|r| r.file_name().and_then(|n| n.to_str()).map(String::from))
    {
        left_spans.push(Span::styled(
            format!(" {}", proj_name),
            Style::new().fg(Color::White),
        ));
    }

    if app.busy {
        let cmd = &CMDS[app.sel_cmd];
        left_spans.push(Span::styled(
            format!(" Executing: {} ", cmd.label),
            Style::new().fg(Color::Yellow),
        ));
    }

    let right_text = " [tab/shift+tab]:Cycle  [↑/↓]:Nav  [enter]:Exec  [r]:Refresh  [pgUp/Dn]:Scroll  [esc]:Quit ";

    let safe_right_len = right_text.len() as u16;
    let [left_area, right_area] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(safe_right_len.min(area.width.saturating_sub(1))),
    ])
    .areas(area);

    frame.render_widget(
        Paragraph::new(Line::from(left_spans))
            .style(Style::new().bg(Color::DarkGray).fg(Color::White)),
        left_area,
    );

    frame.render_widget(
        Paragraph::new(Line::from(right_text))
            .style(Style::new().bg(Color::DarkGray).fg(Color::Gray))
            .alignment(Alignment::Right),
        right_area,
    );
}

// Command Executors

async fn exec_init(root: Option<PathBuf>, raw: String) {
    let parent = root
        .as_deref()
        .and_then(|r| r.parent())
        .unwrap_or_else(|| std::path::Path::new("."));
    let name = raw.split_whitespace().next().unwrap_or("new-project");
    let target = parent.join(name);

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
    let force = raw.split_whitespace().any(|w| w == "--force");

    match ProjectContext::load(&root) {
        Ok(ctx) => {
            if let Err(e) = compiler::compile(&ctx, force).await {
                error!("Build failed: {e}");
            }
        }
        Err(e) => error!("{e}"),
    }
}

async fn exec_lint(root: Option<PathBuf>, _raw: String) {
    let Some(root) = root else {
        error!("No project selected");
        return;
    };
    match ProjectContext::load(&root) {
        Ok(ctx) => doctor::lint_all(&ctx).emit(),
        Err(e) => error!("{e}"),
    }
}

async fn exec_status(root: Option<PathBuf>, _raw: String) {
    let Some(root) = root else {
        error!("No project selected");
        return;
    };
    match ProjectContext::load(&root) {
        Ok(ctx) => project::print_status(&ctx),
        Err(e) => error!("{e}"),
    }
}

async fn exec_doctor(root: Option<PathBuf>, _raw: String) {
    let Some(root) = root else {
        error!("No project selected");
        return;
    };
    match ProjectContext::load(&root) {
        Ok(ctx) => match doctor::run(&ctx) {
            Ok(o) => {
                if !o.has_errors() && !o.has_warnings() {
                    info!("Doctor check complete — no issues found");
                }
            }
            Err(e) => error!("Doctor failed: {e}"),
        },
        Err(e) => error!("{e}"),
    }
}

async fn exec_deploy(root: Option<PathBuf>, raw: String) {
    let Some(root) = root else {
        error!("No project selected");
        return;
    };
    let dry_run = raw.split_whitespace().any(|w| w == "--dry-run");

    match ProjectContext::load(&root) {
        Ok(ctx) => match deploy::deploy(&ctx, dry_run).await {
            Ok(msg) => info!("{msg}"),
            Err(e) => error!("Deploy failed: {e}"),
        },
        Err(e) => error!("{e}"),
    }
}

async fn exec_rollback(root: Option<PathBuf>, raw: String) {
    let Some(root) = root else {
        error!("No project selected");
        return;
    };
    let id = raw.split_whitespace().next().unwrap_or("");
    if id.is_empty() {
        error!("No deployment ID provided");
        return;
    }

    match ProjectContext::load(&root) {
        Ok(ctx) => match deploy::rollback(&ctx, id).await {
            Ok(msg) => info!("{msg}"),
            Err(e) => error!("Rollback failed: {e}"),
        },
        Err(e) => error!("{e}"),
    }
}

async fn exec_clean(root: Option<PathBuf>, _raw: String) {
    let Some(root) = root else {
        error!("No project selected");
        return;
    };
    match ProjectContext::load(&root) {
        Ok(ctx) => match ctx.clean() {
            Ok(()) => info!("Cleaned build artifacts"),
            Err(e) => error!("Clean failed: {e}"),
        },
        Err(e) => error!("{e}"),
    }
}
