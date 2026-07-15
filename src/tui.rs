use std::path::PathBuf;
use std::time::Duration;

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};
use tokio::sync::mpsc;

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

pub async fn run_tui() -> Result<(), CiteError> {
    let mut terminal = ratatui::init();
    let _guard = TerminalGuard;
    terminal
        .clear()
        .map_err(|e| CiteError::Config(format!("{e}")))?;

    let cwd = std::env::current_dir().unwrap_or_default();
    let mut app = AppState::new(&cwd);

    loop {
        terminal
            .draw(|f| render(f, &app))
            .map_err(|e| CiteError::Config(format!("{e}")))?;

        tokio::select! {
            biased;
            // 1. Prioritize receiving task results
            Some(lines) = app.rx.recv() => {
                app.output.extend(lines);
                app.output.push(String::new());
                app.busy = false;
                app.refresh_projects();
                app.scroll = app.output.len().saturating_sub(1); // Auto-scroll to bottom
            }
            // 2. Handle UI events with a small sleep to yield to tokio runtime
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                if event::poll(Duration::from_millis(10)).map_err(|e| CiteError::Config(format!("{e}")))? {
                    if let Event::Key(key) = event::read().map_err(|e| CiteError::Config(format!("{e}")))? {
                        if key.kind == KeyEventKind::Press {
                            if key.code == KeyCode::Esc || (key.code == KeyCode::Char('q') && !matches!(app.focus, Focus::Args)) {
                                break;
                            }
                            app.handle_key(key);
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

// Data types

pub struct Cmd {
    pub label: &'static str,
    pub desc: &'static str,
    pub args_hint: &'static str,
    pub needs_project: bool,
}

pub const CMDS: &[Cmd] = &[
    Cmd {
        label: "Init",
        desc: "Create a new project with starter files",
        args_hint: "<name>",
        needs_project: false,
    },
    Cmd {
        label: "Build",
        desc: "Execute the compiler protocol and build artifact",
        args_hint: "[--force]",
        needs_project: true,
    },
    Cmd {
        label: "Lint",
        desc: "Run linting rules (naming, style, word counts)",
        args_hint: "",
        needs_project: true,
    },
    Cmd {
        label: "Status",
        desc: "Show project health, validation, and sync state",
        args_hint: "",
        needs_project: true,
    },
    Cmd {
        label: "Doctor",
        desc: "Diagnose common project issues and configuration",
        args_hint: "",
        needs_project: true,
    },
    Cmd {
        label: "Deploy",
        desc: "Deploy the built project to Supabase staging",
        args_hint: "[--dry-run]",
        needs_project: true,
    },
    Cmd {
        label: "Rollback",
        desc: "Roll back to the previous deployment",
        args_hint: "<deployment id>",
        needs_project: true,
    },
    Cmd {
        label: "Clean",
        desc: "Remove build artifacts, cache, and temp files",
        args_hint: "",
        needs_project: true,
    },
];

#[derive(Clone, Copy, PartialEq)]
pub enum Focus {
    Projects,
    Commands,
    Args,
    Output,
}

impl Focus {
    fn label(self) -> &'static str {
        match self {
            Focus::Projects => "Projects",
            Focus::Commands => "Commands",
            Focus::Args => "Args",
            Focus::Output => "Output",
        }
    }
}

pub struct AppState {
    cwd: PathBuf,
    pub roots: Vec<PathBuf>,
    pub sel_project: usize,
    pub focus: Focus,
    pub sel_cmd: usize,
    pub output: Vec<String>,
    pub scroll: usize,
    pub busy: bool,
    pub arg_input: String,
    rx: mpsc::Receiver<Vec<String>>,
    tx: mpsc::Sender<Vec<String>>,
}

impl AppState {
    pub fn new(cwd: &PathBuf) -> Self {
        let (tx, rx) = mpsc::channel(32);
        let roots = Self::discover(cwd);

        Self {
            cwd: cwd.clone(),
            roots,
            sel_project: 0,
            focus: Focus::Commands,
            sel_cmd: 0,
            output: vec![],
            scroll: 0,
            busy: false,
            arg_input: String::new(),
            rx,
            tx,
        }
    }

    fn discover(cwd: &PathBuf) -> Vec<PathBuf> {
        let mut r = project::discover_projects(cwd);
        r.sort();
        r
    }

    fn refresh_projects(&mut self) {
        self.cwd = std::env::current_dir().unwrap_or_else(|_| self.cwd.clone());
        self.roots = Self::discover(&self.cwd);
        self.sel_project = self.sel_project.min(self.roots.len().saturating_sub(1));
    }

    pub fn project_names(&self) -> Vec<String> {
        if self.roots.is_empty() {
            return vec![];
        }
        self.roots
            .iter()
            .map(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?")
                    .to_string()
            })
            .collect()
    }

    fn selected_root(&self) -> Option<PathBuf> {
        self.roots.get(self.sel_project).cloned()
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if self.busy {
            return;
        }

        let has_args = !CMDS[self.sel_cmd].args_hint.is_empty();

        match key.code {
            KeyCode::Tab => {
                self.focus = match self.focus {
                    Focus::Projects => Focus::Commands,
                    Focus::Commands => {
                        if has_args {
                            Focus::Args
                        } else {
                            Focus::Output
                        }
                    }
                    Focus::Args => Focus::Output,
                    Focus::Output => Focus::Projects,
                };
            }
            KeyCode::BackTab => {
                self.focus = match self.focus {
                    Focus::Projects => Focus::Output,
                    Focus::Commands => Focus::Projects,
                    Focus::Args => Focus::Commands,
                    Focus::Output => {
                        if has_args {
                            Focus::Args
                        } else {
                            Focus::Commands
                        }
                    }
                };
            }
            KeyCode::Up => match self.focus {
                Focus::Projects => self.sel_project = self.sel_project.saturating_sub(1),
                Focus::Commands | Focus::Args => {
                    self.sel_cmd = self.sel_cmd.saturating_sub(1);
                    if CMDS[self.sel_cmd].args_hint.is_empty() {
                        self.focus = Focus::Commands;
                    }
                }
                Focus::Output => self.scroll = self.scroll.saturating_sub(1),
            },
            KeyCode::Down => match self.focus {
                Focus::Projects => {
                    self.sel_project =
                        (self.sel_project + 1).min(self.roots.len().saturating_sub(1))
                }
                Focus::Commands | Focus::Args => {
                    self.sel_cmd = (self.sel_cmd + 1).min(CMDS.len().saturating_sub(1));
                    if CMDS[self.sel_cmd].args_hint.is_empty() {
                        self.focus = Focus::Commands;
                    }
                }
                Focus::Output => self.scroll = self.scroll.saturating_add(1),
            },
            KeyCode::Enter => match self.focus {
                Focus::Projects => self.focus = Focus::Commands,
                Focus::Commands | Focus::Args => self.start_cmd(),
                _ => {}
            },
            KeyCode::Backspace => {
                if matches!(self.focus, Focus::Args) {
                    self.arg_input.pop();
                }
            }
            KeyCode::Char('r') if !matches!(self.focus, Focus::Args) => {
                self.refresh_projects();
                self.output.clear();
                self.output.push("Refreshed".into());
                self.scroll = 0;
            }
            KeyCode::Char(ch) => {
                if matches!(self.focus, Focus::Args) {
                    self.arg_input.push(ch);
                }
            }
            KeyCode::PageUp => {
                if matches!(self.focus, Focus::Output) {
                    self.scroll = self.scroll.saturating_sub(10);
                }
            }
            KeyCode::PageDown => {
                if matches!(self.focus, Focus::Output) {
                    self.scroll = self.scroll.saturating_add(10);
                }
            }
            _ => {}
        }
    }

    fn start_cmd(&mut self) {
        let root = self.selected_root();
        let cmd = &CMDS[self.sel_cmd];

        if cmd.needs_project && root.is_none() {
            self.output
                .push("!! No projects found — select or init a project first".into());
            return;
        }

        let raw_args = std::mem::take(&mut self.arg_input);

        let arg_display = if raw_args.is_empty() {
            String::new()
        } else {
            format!(" ({raw_args})")
        };

        self.output.push(format!(
            ">> {}{} {}",
            cmd.label,
            arg_display,
            root.as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        ));

        self.busy = true;
        let tx = self.tx.clone();
        let idx = self.sel_cmd;

        tokio::spawn(async move {
            let result = match idx {
                0 => exec_init(root, &raw_args).await,
                1 => exec_build(root, &raw_args).await,
                2 => exec_lint(root).await,
                3 => exec_status(root).await,
                4 => exec_doctor(root).await,
                5 => exec_deploy(root, &raw_args).await,
                6 => exec_rollback(root, &raw_args).await,
                7 => exec_clean(root).await,
                _ => vec!["Unknown command".into()],
            };
            let _ = tx.send(result).await;
        });
    }
}

// Rendering

fn block(title: &str, focused: bool) -> Block<'_> {
    let border_style = if focused {
        Style::new().fg(Color::Cyan).bold()
    } else {
        Style::new().fg(Color::DarkGray)
    };
    Block::default()
        .borders(Borders::ALL)
        .title(format!(" {title} "))
        .border_style(border_style)
}

fn render(frame: &mut Frame, app: &AppState) {
    let [header, body, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    render_header(frame, header);
    render_body(frame, body, app);
    render_statusbar(frame, status, app);
}

fn render_header(frame: &mut Frame, area: Rect) {
    let t = Line::from(vec![Span::styled(
        format!("v{}", env!("CARGO_PKG_VERSION")),
        Style::new().fg(Color::DarkGray),
    )]);
    frame.render_widget(Paragraph::new(t), area);
}

fn render_body(frame: &mut Frame, area: Rect, app: &AppState) {
    let [left, right] = Layout::horizontal([Constraint::Max(25), Constraint::Fill(1)]).areas(area);

    let [top, bot] = Layout::vertical([Constraint::Max(12), Constraint::Fill(1)]).areas(right);

    let [cmd_list, cmd_doc] =
        Layout::horizontal([Constraint::Max(25), Constraint::Fill(1)]).areas(top);

    render_projects(frame, left, app);
    render_cmds(frame, cmd_list, app);
    render_cmd_doc(frame, cmd_doc, app);
    render_output(frame, bot, app);
}

fn render_projects(frame: &mut Frame, area: Rect, app: &AppState) {
    let is_focused = matches!(app.focus, Focus::Projects);
    let items: Vec<ListItem> = app
        .project_names()
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let is_selected = i == app.sel_project;
            let prefix = if is_selected { "▸ " } else { "  " };
            let style = if is_selected && is_focused {
                Style::new().fg(Color::Black).bg(Color::Cyan)
            } else if is_selected {
                Style::new().fg(Color::Cyan)
            } else {
                Style::new().fg(Color::Gray)
            };
            ListItem::new(Line::from(Span::styled(format!("{prefix}{name}"), style)))
        })
        .collect();

    let title = format!("Projects ({})", app.roots.len());
    frame.render_widget(List::new(items).block(block(&title, is_focused)), area);
}

fn render_cmds(frame: &mut Frame, area: Rect, app: &AppState) {
    let is_focused = matches!(app.focus, Focus::Commands);
    let items: Vec<ListItem> = CMDS
        .iter()
        .enumerate()
        .map(|(i, cmd)| {
            let is_selected = i == app.sel_cmd;
            let prefix = if is_selected { "▸ " } else { "  " };
            let style = if is_selected && is_focused {
                Style::new().fg(Color::Black).bg(Color::Cyan)
            } else if is_selected {
                Style::new().fg(Color::Cyan)
            } else {
                Style::new().fg(Color::Gray)
            };
            ListItem::new(Line::from(Span::styled(
                format!("{prefix}{}", cmd.label),
                style,
            )))
        })
        .collect();

    frame.render_widget(List::new(items).block(block("Commands", is_focused)), area);
}

fn render_cmd_doc(frame: &mut Frame, area: Rect, app: &AppState) {
    let cmd = &CMDS[app.sel_cmd];
    let is_focused = matches!(app.focus, Focus::Args);
    let has_args = !cmd.args_hint.is_empty();

    let mut lines = vec![
        Line::from(Span::styled(cmd.label, Style::new().bold().fg(Color::Cyan))),
        Line::from(""),
        Line::from(cmd.desc),
    ];

    if has_args {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Arguments:",
            Style::new().bold().fg(Color::Gray),
        )));
        lines.push(Line::from(Span::raw(format!("  {}", cmd.args_hint))));

        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("Input: ", Style::new().bold().fg(Color::Gray)),
            Span::styled(
                if app.arg_input.is_empty() {
                    "Type arguments here..."
                } else {
                    &app.arg_input
                },
                if is_focused {
                    Style::new()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::UNDERLINED)
                } else {
                    Style::new().fg(Color::Gray)
                },
            ),
        ]));
    }

    frame.render_widget(
        Paragraph::new(Text::from(lines)).block(block("Details", is_focused)),
        area,
    );
}

fn render_output(frame: &mut Frame, area: Rect, app: &AppState) {
    let lines: Vec<Line> = app.output.iter().map(|l| Line::raw(l.clone())).collect();

    let visible_lines = area.height.saturating_sub(3) as usize;
    let max_scroll = app.output.len().saturating_sub(visible_lines);
    let scroll_y = app.scroll.min(max_scroll) as u16;

    let para = Paragraph::new(Text::from(lines))
        .block(block("Output", matches!(app.focus, Focus::Output)))
        .scroll((scroll_y, 0))
        .wrap(Wrap { trim: false });

    frame.render_widget(para, area);
}

fn render_statusbar(frame: &mut Frame, area: Rect, app: &AppState) {
    let mut left_spans = vec![];

    if app.busy {
        left_spans.push(Span::styled(
            " RUNNING ",
            Style::new().fg(Color::Black).bg(Color::Yellow),
        ));
    } else {
        left_spans.push(Span::styled(
            " READY ",
            Style::new().fg(Color::Black).bg(Color::Green),
        ));
    }

    left_spans.push(Span::raw("  "));
    left_spans.push(Span::styled(
        app.focus.label(),
        Style::new().bold().fg(Color::Cyan),
    ));

    if !app.roots.is_empty() {
        left_spans.push(Span::raw("  "));
        let proj_name = app
            .project_names()
            .get(app.sel_project)
            .cloned()
            .unwrap_or_default();
        left_spans.push(Span::styled(proj_name, Style::new().fg(Color::Gray)));
    }

    if app.busy {
        let cmd = &CMDS[app.sel_cmd];
        left_spans.push(Span::raw("  "));
        left_spans.push(Span::styled(
            format!("Executing: {}", cmd.label),
            Style::new().fg(Color::Yellow),
        ));
    }

    let right_text = " [Tab/Shift+Tab]:Cycle  [↑/↓]:Nav  [Enter]:Exec  [r]:Refresh  [PgUp/Dn]:Scroll  [Esc/q]:Quit ";

    let [left_area, right_area] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(right_text.len() as u16),
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

// Command executors

async fn exec_init(root: Option<PathBuf>, raw: &str) -> Vec<String> {
    let parent = root
        .as_deref()
        .and_then(|r| r.parent())
        .unwrap_or_else(|| std::path::Path::new("."));

    let name = raw.split_whitespace().next().unwrap_or("new-project");
    let target = parent.join(name);

    match scaffold::init_project(name, &target) {
        Ok(report) => {
            let mut l = report.to_lines();
            l.push(format!("Project '{name}' created at {}", target.display()));
            l
        }
        Err(e) => vec![format!("Init failed: {e}")],
    }
}

async fn exec_build(root: Option<PathBuf>, raw: &str) -> Vec<String> {
    let Some(root) = root else {
        return vec!["No project selected".into()];
    };

    let force = raw.split_whitespace().any(|w| w == "--force");

    match ProjectContext::load(&root) {
        Ok(ctx) => match compiler::compile(&ctx, force).await {
            Ok(r) => r.to_lines(),
            Err(e) => vec![format!("Build failed: {e}")],
        },
        Err(e) => vec![format!("{e}")],
    }
}

async fn exec_lint(root: Option<PathBuf>) -> Vec<String> {
    let Some(root) = root else {
        return vec!["No project selected".into()];
    };

    match ProjectContext::load(&root) {
        Ok(ctx) => {
            let mut lines = doctor::lint_all(&ctx).to_lines();
            if lines.is_empty() {
                lines.push("Lint complete — no issues found".into());
            }
            lines
        }
        Err(e) => vec![format!("{e}")],
    }
}

async fn exec_status(root: Option<PathBuf>) -> Vec<String> {
    let Some(root) = root else {
        return vec!["No project selected".into()];
    };

    match ProjectContext::load(&root) {
        Ok(ctx) => {
            let mut lines = Vec::new();
            lines.push(format!("Name: {}", ctx.manifest.project.name));
            lines.push(format!("Root: {}", ctx.root.display()));
            lines.push(format!("Artist ID: {}", ctx.manifest.project.artist_id));
            if let Some(b) = &ctx.manifest.backend {
                if let Some(u) = &b.staging_url {
                    lines.push(format!("Staging: {u}"));
                }
            }
            lines.push(format!("Podcasts: {}", ctx.metadata.podcasts.len()));

            let build_path = ctx.build_dir().join("content.json");
            lines.push(if build_path.exists() {
                "Build: exists".into()
            } else {
                "Build: not built".into()
            });
            lines
        }
        Err(e) => vec![format!("{e}")],
    }
}

async fn exec_doctor(root: Option<PathBuf>) -> Vec<String> {
    let Some(root) = root else {
        return vec!["No project selected".into()];
    };

    match ProjectContext::load(&root) {
        Ok(ctx) => match doctor::run(&ctx) {
            Ok(o) => {
                let mut lines = o.to_lines();
                if lines.is_empty() {
                    lines.push("Doctor check complete — no issues found".into());
                }
                lines
            }
            Err(e) => vec![format!("Doctor failed: {e}")],
        },
        Err(e) => vec![format!("{e}")],
    }
}

async fn exec_deploy(root: Option<PathBuf>, raw: &str) -> Vec<String> {
    let Some(root) = root else {
        return vec!["No project selected".into()];
    };

    let dry_run = raw.split_whitespace().any(|w| w == "--dry-run");

    match ProjectContext::load(&root) {
        Ok(ctx) => match deploy::deploy(&ctx, dry_run).await {
            Ok(msg) => vec![msg],
            Err(e) => vec![format!("Deploy failed: {e}")],
        },
        Err(e) => vec![format!("{e}")],
    }
}

async fn exec_rollback(root: Option<PathBuf>, raw: &str) -> Vec<String> {
    let Some(root) = root else {
        return vec!["No project selected".into()];
    };

    let id = raw.split_whitespace().next().unwrap_or("");
    if id.is_empty() {
        return vec!["No deployment ID provided".into()];
    }

    match ProjectContext::load(&root) {
        Ok(ctx) => match deploy::rollback(&ctx, id).await {
            Ok(msg) => vec![msg],
            Err(e) => vec![format!("Rollback failed: {e}")],
        },
        Err(e) => vec![format!("{e}")],
    }
}

async fn exec_clean(root: Option<PathBuf>) -> Vec<String> {
    let Some(root) = root else {
        return vec!["No project selected".into()];
    };

    match ProjectContext::load(&root) {
        Ok(ctx) => match ctx.clean() {
            Ok(()) => vec!["Cleaned build artifacts".into()],
            Err(e) => vec![format!("Clean failed: {e}")],
        },
        Err(e) => vec![format!("{e}")],
    }
}
