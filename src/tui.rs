use anyhow::Result;
use chrono::{Duration, Utc};
use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::execute;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::stdout;

use crate::collectors;
use crate::db::Database;
use crate::models::{DailyRow, ModelPricing, SummaryRow, UsageRecord};
use crate::theme::Theme;

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Summary,
    Daily,
    Weekly,
    Monthly,
    Recent,
    Models,
    Status,
}

impl View {
    const ALL: [View; 7] = [
        View::Summary,
        View::Daily,
        View::Weekly,
        View::Monthly,
        View::Recent,
        View::Models,
        View::Status,
    ];
    fn label(self) -> &'static str {
        match self {
            View::Summary => " Summary ",
            View::Daily => " Daily ",
            View::Weekly => " Weekly ",
            View::Monthly => " Monthly ",
            View::Recent => " Recent ",
            View::Models => " Models ",
            View::Status => " Status ",
        }
    }
    fn number(self) -> u8 {
        match self {
            View::Summary => 1,
            View::Daily => 2,
            View::Weekly => 3,
            View::Monthly => 4,
            View::Recent => 5,
            View::Models => 6,
            View::Status => 7,
        }
    }
}

/// Single source of truth for /-commands: drives the palette and the command
/// handler, so they can never drift apart.
#[derive(Clone, Copy)]
struct CommandSpec {
    name: &'static str,
    args: Option<&'static str>,
    description: &'static str,
    category: &'static str,
}

const COMMANDS: &[CommandSpec] = &[
    CommandSpec { name: "/summary", args: None, description: "summary view", category: "Views" },
    CommandSpec { name: "/daily", args: None, description: "daily view", category: "Views" },
    CommandSpec { name: "/weekly", args: None, description: "weekly view", category: "Views" },
    CommandSpec { name: "/monthly", args: None, description: "monthly view", category: "Views" },
    CommandSpec { name: "/recent", args: None, description: "recent records", category: "Views" },
    CommandSpec { name: "/models", args: None, description: "model pricing list", category: "Views" },
    CommandSpec { name: "/status", args: None, description: "collector detection status", category: "Views" },
    CommandSpec { name: "/days", args: Some("<N>"), description: "set range to last N days", category: "Actions" },
    CommandSpec { name: "/refresh", args: None, description: "reload data from DB", category: "Actions" },
    CommandSpec { name: "/export", args: Some("<csv|json>"), description: "export usage to file", category: "Actions" },
    CommandSpec { name: "/theme", args: Some("<name>"), description: "switch color theme (default, nord, dracula)", category: "Actions" },
    CommandSpec { name: "/quit", args: None, description: "exit", category: "Actions" },
    CommandSpec { name: "/sync", args: None, description: "run in shell: tokentracker sync", category: "Shell" },
    CommandSpec { name: "/reprice", args: None, description: "run in shell: tokentracker reprice", category: "Shell" },
    CommandSpec { name: "/update-pricing", args: None, description: "run in shell: tokentracker update-pricing", category: "Shell" },
    CommandSpec { name: "/serve", args: None, description: "run in shell: tokentracker serve", category: "Shell" },
    CommandSpec { name: "/config", args: None, description: "run in shell: tokentracker config", category: "Shell" },
    CommandSpec { name: "/antigravity", args: None, description: "run in shell: tokentracker antigravity", category: "Shell" },
    CommandSpec { name: "/completions", args: None, description: "run in shell: tokentracker completions", category: "Shell" },
];

/// Prefix match on the first token. Stop suggesting once a full command plus
/// its argument has been typed.
fn match_commands(input: &str) -> Vec<&'static CommandSpec> {
    if !input.starts_with('/') {
        return vec![];
    }
    let typed = input.split(' ').next().unwrap_or("").to_lowercase();
    if input.contains(' ') && COMMANDS.iter().any(|c| c.name == typed) {
        return vec![];
    }
    COMMANDS
        .iter()
        .filter(|c| c.name.starts_with(&typed))
        .collect()
}

const MAX_SUGGESTIONS: usize = 6;

/// Data windows for the views that don't follow `--days`. Single source of
/// truth shared by reload() (which queries with them) and render_title()
/// (which labels them), so the two can never drift apart.
const WEEKLY_WINDOW_WEEKS: u32 = 12;
const MONTHLY_WINDOW_MONTHS: u32 = 6;
const RECENT_RECORD_LIMIT: usize = 100;

/// True when `input` is a full command with its argument typed (e.g.
/// "/days 365"), meaning Enter should run it directly even though the
/// suggestion list is empty.
fn is_fully_typed_command(input: &str) -> bool {
    let trimmed = input.trim();
    let first = trimmed.split_whitespace().next().unwrap_or("");
    let rest = trimmed[first.len()..].trim();
    !first.is_empty() && COMMANDS.iter().any(|c| c.name == first) && !rest.is_empty()
}

fn fmt_int(n: i64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out.chars().rev().collect()
}

fn fmt_cost(cost: f64) -> String {
    if cost == 0.0 {
        "0".to_string()
    } else if cost < 0.01 {
        format!("${:.4}", cost)
    } else {
        format!("${:.2}", cost)
    }
}

fn free_note(model: &str) -> String {
    if model.ends_with("-free") {
        format!("{model} (free)")
    } else {
        model.to_string()
    }
}

#[derive(Default, Copy, Clone)]
struct Chip {
    label: &'static str,
    rect: Rect,
    hovered: bool,
}

pub struct App {
    days: u32,
    summary: Vec<SummaryRow>,
    daily: Vec<DailyRow>,
    weekly: Vec<DailyRow>,
    monthly: Vec<DailyRow>,
    recent: Vec<UsageRecord>,
    models: Vec<ModelPricing>,
    status: Vec<collectors::LocalCollectorStatus>,
    view: View,
    list_state: ListState,
    command: String,
    palette_open: bool,
    /// Index into the matched command list, not the display lines.
    suggestion_index: usize,
    quit: bool,
    status_msg: String,
    /// Bottom action chips; rects filled on every draw.
    chips: [Chip; 3],
    /// Tab chips; rects filled on every draw, same Chip mechanism.
    tabs: [Chip; 7],
    /// Set when an overlay (export result / shell hint) is showing.
    overlay: Option<OverlayKind>,
    /// Popup rect + row->command mapping from the last palette draw, for
    /// mouse hit-testing.
    palette_rect: Option<Rect>,
    palette_rows: Vec<Option<usize>>,
    /// Overlay popup rect from the last draw, for mouse hit-testing.
    overlay_rect: Option<Rect>,
    /// Current color theme.
    theme: Theme,
}

enum OverlayKind {
    Export { message: String },
    Shell { message: String },
}

impl App {
    fn new(db: &Database, days: u32) -> Result<Self> {
        let mut app = Self {
            days,
            summary: Vec::new(),
            daily: Vec::new(),
            weekly: Vec::new(),
            monthly: Vec::new(),
            recent: Vec::new(),
            models: Vec::new(),
            status: Vec::new(),
            view: View::Summary,
            list_state: ListState::default(),
            command: String::new(),
            palette_open: false,
            suggestion_index: 0,
            quit: false,
            status_msg: "Ready".to_string(),
            chips: [
                Chip { label: "Refresh", ..Default::default() },
                Chip { label: "Export", ..Default::default() },
                Chip { label: "Quit", ..Default::default() },
            ],
            tabs: View::ALL.map(|v| Chip { label: v.label().trim(), ..Default::default() }),
            overlay: None,
            palette_rect: None,
            palette_rows: Vec::new(),
            overlay_rect: None,
            theme: Theme::default(),
        };
        app.reload(db);
        Ok(app)
    }

    fn reload(&mut self, db: &Database) {
        self.summary = db.query_summary(self.days, None, None).unwrap_or_default();
        self.daily = db
            .query_daily(self.days, None, None, None, None)
            .unwrap_or_default();
        self.weekly = db
            .query_weekly(WEEKLY_WINDOW_WEEKS, None, None, None, None)
            .unwrap_or_default();
        self.monthly = db
            .query_monthly(MONTHLY_WINDOW_MONTHS, None, None, None, None)
            .unwrap_or_default();
        self.recent = db
            .query_detail(None, None, None, None, Some(RECENT_RECORD_LIMIT))
            .unwrap_or_default();
        self.models = crate::costs::get_model_pricing(None);
        self.status = collectors::local_collector_statuses();
        self.list_state.select(Some(0));
    }

    fn set_view(&mut self, v: View) {
        self.view = v;
        self.list_state.select(Some(0));
    }

    fn view_len(&self) -> usize {
        match self.view {
            View::Summary => 0,
            View::Daily => self.daily.len() * 2,
            View::Weekly => self.weekly.len() * 2,
            View::Monthly => self.monthly.len() * 2,
            View::Recent => self.recent.len(),
            View::Models => self.models.len(),
            View::Status => self.status.len(),
        }
    }

    fn move_selection(&mut self, delta: i64) {
        let len = self.view_len();
        if len == 0 {
            return;
        }
        let cur = self.list_state.selected().unwrap_or(0) as i64;
        let next = (cur + delta).clamp(0, (len - 1) as i64) as usize;
        self.list_state.select(Some(next));
    }

    /// Accept the highlighted suggestion into the input (does not run it) —
    /// commands with arguments get completed, arg-less ones run immediately.
    /// Returns the submitted command string, or None when the arg was filled
    /// or the palette list is empty (caller falls back to running the raw input).
    fn commit_palette_selection(&mut self) -> Option<String> {
        let trimmed = self.command.trim();
        // If user has typed a bare command that requires args (e.g. "/theme" or "/theme "),
        // submit it directly so execute_command can show the usage hint.
        if let Some(cmd) = COMMANDS.iter().find(|c| c.name == trimmed && c.args.is_some()) {
            return Some(cmd.name.to_string());
        }
        let matches = match_commands(&self.command);
        if matches.is_empty() {
            return None;
        }
        self.suggestion_index = self.suggestion_index.min(matches.len() - 1);
        let pick = *matches[self.suggestion_index];
        if pick.args.is_some() {
            // Complete into the input and stay in the palette for the arg.
            self.command = format!("{} ", pick.name);
            self.suggestion_index = 0;
            return None;
        }
        Some(pick.name.to_string())
    }

    fn execute_command(&mut self, db: &Database, raw: &str) {
        let cmd = raw.trim().to_lowercase();
        let command = cmd.as_str();
        match command {
            "/summary" => self.set_view(View::Summary),
            "/daily" => self.set_view(View::Daily),
            "/weekly" => self.set_view(View::Weekly),
            "/monthly" => self.set_view(View::Monthly),
            "/recent" => self.set_view(View::Recent),
            "/models" => self.set_view(View::Models),
            "/status" => self.set_view(View::Status),
            "/quit" => self.quit = true,
            "/refresh" => {
                self.reload(db);
                self.status_msg = "Refreshed".to_string();
            }
            "/export" => {
                self.status_msg = "usage: /export csv|json".to_string();
            }
            _ if command.starts_with("/days ") => {
                let n = command.split_whitespace().nth(1).and_then(|s| s.parse().ok());
                match n {
                    Some(n) => {
                        self.days = n;
                        self.reload(db);
                        self.status_msg = format!("Range: last {n} days");
                    }
                    None => self.status_msg = "usage: /days <N>".to_string(),
                }
            }
            _ if command.starts_with("/export ") => {
                let fmt = command.split_whitespace().nth(1).unwrap_or("csv");
                let since = Utc::now() - Duration::days(self.days as i64);
                let since_str = since.format("%Y-%m-%d").to_string();
                let rows = db.query_detail(None, None, Some(&since_str), None, None);
                match rows {
                    Ok(rows) => {
                        let content = if fmt == "csv" {
                            crate::display::to_csv(&rows)
                        } else {
                            crate::display::to_json(&rows).unwrap_or_default()
                        };
                        let path = format!("tokentracker-export-{}.{}", self.days, fmt);
                        match std::fs::write(&path, &content) {
                            Ok(_) => {
                                let msg = format!("Exported {} records to {path}", rows.len());
                                self.status_msg = msg.clone();
                                self.overlay = Some(OverlayKind::Export { message: msg });
                            }
                            Err(e) => self.status_msg = format!("export failed: {e}"),
                        }
                    }
                    Err(e) => self.status_msg = format!("export failed: {e}"),
                }
            }
            "/sync" | "/reprice" | "/update-pricing" | "/serve" | "/config" | "/antigravity"
            | "/completions" => {
                let shell = format!("tokentracker {}", command.trim_start_matches('/'));
                let msg = format!("not runnable in-TUI — run: {shell}");
                self.status_msg = msg.clone();
                self.overlay = Some(OverlayKind::Shell { message: msg });
            }
            _ if command.starts_with("/theme") => {
                let parts: Vec<&str> = command.split_whitespace().collect();
                if parts.len() == 1 {
                    // No argument - show available themes
                    self.status_msg = format!("available themes: {}", Theme::available_names().join(", "));
                } else if let Some(theme) = Theme::by_name(parts[1]) {
                    self.theme = theme;
                    self.status_msg = format!("theme set to: {}", parts[1]);
                } else {
                    self.status_msg = format!("unknown theme: {} (available: {})", parts[1], Theme::available_names().join(", "));
                }
            }
            _ => {
                self.status_msg = format!("unknown command: {cmd}");
            }
        }
        self.command.clear();
        self.palette_open = false;
    }
}

/// Range label for the title bar, derived from the active view. Views
/// without a time dimension (Models, Status) return None so the label is
/// omitted entirely.
fn range_label(view: View, days: u32) -> Option<String> {
    match view {
        View::Summary | View::Daily => Some(format!("last {days} days")),
        View::Weekly => Some(format!("last {WEEKLY_WINDOW_WEEKS} weeks")),
        View::Monthly => Some(format!("last {MONTHLY_WINDOW_MONTHS} months")),
        View::Recent => Some(format!("last {RECENT_RECORD_LIMIT} records")),
        View::Models | View::Status => None,
    }
}

fn render_title(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let mut spans: Vec<Span> = vec![Span::styled(
        " tokentracker ",
        Style::default()
            .fg(app.theme.selection_fg)
            .bg(app.theme.selection_bg)
            .add_modifier(Modifier::BOLD),
    )];
    if let Some(label) = range_label(app.view, app.days) {
        spans.push(Span::styled(
            format!(" {label} "),
            Style::default().fg(app.theme.muted),
        ));
    }
    spans.push(Span::styled(
        format!(" - {}", app.status_msg),
        Style::default().fg(app.theme.cost),
    ));
    let title = Line::from(spans);
    f.render_widget(Paragraph::new(title).alignment(ratatui::layout::Alignment::Left), area);
}

fn render_tabs(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let n = View::ALL.len() as u32;
    let widths: Vec<Constraint> = View::ALL.iter().map(|_| Constraint::Ratio(1, n)).collect();
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(widths)
        .split(area);
    for (i, v) in View::ALL.iter().enumerate() {
        app.tabs[i].rect = chunks[i];
        let selected = *v == app.view;
        let style = if selected {
            Style::default()
                .fg(app.theme.selection_fg)
                .bg(app.theme.selection_bg)
                .add_modifier(Modifier::BOLD)
        } else if app.tabs[i].hovered {
            Style::default().fg(app.theme.selection_fg).bg(app.theme.hover_bg)
        } else {
            Style::default().fg(app.theme.muted)
        };
        let text = Paragraph::new(Line::from(Span::styled(v.label().trim(), style)))
            .alignment(ratatui::layout::Alignment::Center);
        f.render_widget(text, chunks[i]);
    }
}

fn render_summary(f: &mut ratatui::Frame, rows: &[SummaryRow], theme: &Theme, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!("{:<13}{:<30}{:>12}{:>12}{:>10}{:>9}", "PROVIDER", "MODEL", "INPUT", "OUTPUT", "COST", "RECORDS"),
        Style::default().fg(theme.header).add_modifier(Modifier::BOLD),
    )));
    for r in rows {
        lines.push(Line::from(vec![
            Span::raw(format!("{:<13}", r.provider)),
            Span::raw(format!("{:<30}", free_note(&r.model))),
            Span::raw(format!("{:>12}", fmt_int(r.total_input))),
            Span::raw(format!("{:>12}", fmt_int(r.total_output))),
            Span::styled(format!("{:>10}", fmt_cost(r.total_cost)), Style::default().fg(theme.cost)),
            Span::raw(format!("{:>9}", fmt_int(r.record_count))),
        ]));
    }
    if !rows.is_empty() {
        let total_input: i64 = rows.iter().map(|r| r.total_input).sum();
        let total_output: i64 = rows.iter().map(|r| r.total_output).sum();
        let total_cost: f64 = rows.iter().map(|r| r.total_cost).sum();
        let record_count: i64 = rows.iter().map(|r| r.record_count).sum();
        lines.push(Line::from("-".repeat(86)));
        lines.push(Line::from(vec![
            Span::styled(format!("{:<13}", "TOTAL"), Style::default().fg(theme.text).add_modifier(Modifier::BOLD)),
            Span::raw(format!("{:<30}", "")),
            Span::styled(format!("{:>12}", fmt_int(total_input)), Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(format!("{:>12}", fmt_int(total_output)), Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(format!("{:>10}", fmt_cost(total_cost)), Style::default().fg(theme.cost).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{:>9}", fmt_int(record_count)), Style::default().add_modifier(Modifier::BOLD)),
        ]));
    }
    let para = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Usage Summary"))
        .wrap(Wrap { trim: false });
    f.render_widget(para, area);
}

fn render_period_rows(f: &mut ratatui::Frame, rows: &[DailyRow], title: &str, theme: &Theme, state: &mut ListState, area: Rect) {
    let mut items: Vec<ListItem> = Vec::new();
    for r in rows {
        items.push(ListItem::new(Line::from(vec![
            Span::raw(format!("{:<12}", r.date)),
            Span::styled(format!("{:>12}", fmt_int(r.total_input)), Style::default().fg(theme.input)),
            Span::raw(" in / "),
            Span::styled(format!("{:>12}", fmt_int(r.total_output)), Style::default().fg(theme.positive)),
            Span::raw(" out / "),
            Span::styled(fmt_cost(r.total_cost), Style::default().fg(theme.cost)),
        ])));
        items.push(ListItem::new(Line::from(Span::styled(
            format!("    models: {}", r.models.join(", ")),
            Style::default().fg(theme.muted),
        ))));
    }
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(
            Style::default().bg(theme.list_selection_bg).add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");
    f.render_stateful_widget(list, area, state);
}

fn render_recent(f: &mut ratatui::Frame, rows: &[UsageRecord], theme: &Theme, state: &mut ListState, area: Rect) {
    let mut items: Vec<ListItem> = Vec::new();
    for r in rows {
        items.push(ListItem::new(Line::from(vec![
            Span::raw(format!("{:<20}", r.recorded_at.chars().take(16).collect::<String>())),
            Span::raw(format!("{:<12}", r.provider)),
            Span::raw(format!("{:<30}", free_note(&r.model))),
            Span::styled(format!("{:>10}", fmt_int(r.input_tokens)), Style::default().fg(theme.input)),
            Span::raw(" in / "),
            Span::styled(format!("{:>10}", fmt_int(r.output_tokens)), Style::default().fg(theme.positive)),
            Span::raw(" / "),
            Span::styled(
                r.cost_usd.map(fmt_cost).unwrap_or_else(|| "-".to_string()),
                Style::default().fg(theme.cost),
            ),
        ])));
    }
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Recent Records"))
        .highlight_style(
            Style::default().bg(theme.list_selection_bg).add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");
    f.render_stateful_widget(list, area, state);
}

fn render_models(f: &mut ratatui::Frame, rows: &[ModelPricing], theme: &Theme, state: &mut ListState, area: Rect) {
    let mut items: Vec<ListItem> = Vec::new();
    items.push(ListItem::new(Line::from(Span::styled(
        format!("{:<13}{:<30}{:>12}{:>12}", "PROVIDER", "MODEL", "$/MTok IN", "$/MTok OUT"),
        Style::default().fg(theme.header).add_modifier(Modifier::BOLD),
    ))));
    for m in rows {
        items.push(ListItem::new(Line::from(vec![
            Span::raw(format!("{:<13}", m.provider)),
            Span::raw(format!("{:<30}", m.model)),
            Span::styled(format!("{:>12}", fmt_cost(m.input_per_mtok)), Style::default().fg(theme.input)),
            Span::styled(format!("{:>12}", fmt_cost(m.output_per_mtok)), Style::default().fg(theme.positive)),
        ])));
    }
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Model Pricing"))
        .highlight_style(
            Style::default().bg(theme.list_selection_bg).add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");
    f.render_stateful_widget(list, area, state);
}

fn render_status(f: &mut ratatui::Frame, rows: &[collectors::LocalCollectorStatus], theme: &Theme, state: &mut ListState, area: Rect) {
    let mut items: Vec<ListItem> = Vec::new();
    for s in rows {
        let detected = s.state == collectors::LocalCollectorState::Detected;
        items.push(ListItem::new(Line::from(vec![
            Span::styled(
                format!("{:<14}", s.name),
                if detected {
                    Style::default().fg(theme.positive).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.muted)
                },
            ),
            Span::styled(
                if detected { "detected" } else { "not found" },
                if detected {
                    Style::default().fg(theme.positive)
                } else {
                    Style::default().fg(theme.negative)
                },
            ),
            Span::styled(format!("  {}", s.path.display()), Style::default().fg(theme.muted)),
        ])));
        if let Some(note) = s.note {
            items.push(ListItem::new(Line::from(Span::styled(
                format!("    {note}"),
                Style::default().fg(theme.muted),
            ))));
        }
    }
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Collector Status"))
        .highlight_style(
            Style::default().bg(theme.list_selection_bg).add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");
    f.render_stateful_widget(list, area, state);
}

/// Compute the visible slice of the suggestion list so the currently selected
/// command is always on screen. The list is a flat line list (category headers
/// and command rows interleaved); the window is a slice over those lines.
/// Returns `(start, content_rows, hidden_above, hidden_below)` where the
/// hidden counts are command entries cut off above/below the window.
fn suggestion_window(
    sel_row: usize,
    total: usize,
    rows: &[Option<usize>],
    max: usize,
) -> (usize, usize, usize, usize) {
    let count_some = |r: &[Option<usize>]| r.iter().filter(|x| x.is_some()).count();
    if total <= max {
        return (0, total, 0, 0);
    }
    let start_for = |content: usize, sel: usize| -> usize {
        ((sel as isize - (content as isize / 2)).clamp(0, (total - content) as isize)) as usize
    };

    let mut content = max;
    let mut start = start_for(content, sel_row);
    let mut above = count_some(&rows[..start]);
    let mut below = count_some(&rows[start + content..]);
    for _ in 0..3 {
        let markers = (above > 0) as usize + (below > 0) as usize;
        let new_content = max.saturating_sub(markers).max(1);
        let new_start = start_for(new_content, sel_row);
        let new_above = count_some(&rows[..new_start]);
        let new_below = count_some(&rows[new_start + new_content..]);
        if new_content == content && new_start == start {
            break;
        }
        content = new_content;
        start = new_start;
        above = new_above;
        below = new_below;
    }
    (start, content, above, below)
}

/// Always-rendered command bar row. Shows the "/" input while the palette is
/// open (with a live cursor) and a dim hint otherwise.
fn render_command_bar(f: &mut ratatui::Frame, app: &App, area: Rect) {
    if app.palette_open {
        let line = Line::from(vec![
            Span::styled("> ", Style::default().fg(app.theme.accent).add_modifier(Modifier::BOLD)),
            Span::styled(
                app.command.clone(),
                Style::default().fg(app.theme.text).add_modifier(Modifier::BOLD),
            ),
        ]);
        f.render_widget(Paragraph::new(line), area);
        let cursor_x = area.x + 2 + app.command.chars().count() as u16;
        if cursor_x < area.x + area.width {
            f.set_cursor_position(Position::new(cursor_x, area.y));
        }
    } else {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " [/] commands ",
                Style::default().fg(app.theme.muted),
            ))),
            area,
        );
    }
}

fn render_suggestions(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    if !app.palette_open {
        app.palette_rect = None;
        app.palette_rows.clear();
        return;
    }
    app.palette_rect = Some(area);
    f.render_widget(Clear, area);
    app.palette_rows.clear();

    let matches = match_commands(&app.command);
    if matches.is_empty() {
        app.palette_rows.push(None);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "no matching commands",
                Style::default().fg(app.theme.negative),
            ))),
            area,
        );
        return;
    }

    // Full flat line list (headers + commands) with a parallel map marking
    // which content row holds which command.
    let sel = app.suggestion_index.min(matches.len() - 1);
    let mut lines: Vec<Line> = Vec::new();
    let mut rows: Vec<Option<usize>> = Vec::new();
    let mut sel_row = 0;
    let mut last_category = "";
    for (i, m) in matches.iter().enumerate() {
        if m.category != last_category {
            last_category = m.category;
            rows.push(None);
            lines.push(Line::from(Span::styled(
                format!("  {}", m.category),
                Style::default().fg(app.theme.muted).add_modifier(Modifier::ITALIC),
            )));
        }
        let selected = i == sel;
        if selected {
            sel_row = lines.len();
        }
        let args = m.args.map(|a| format!(" {a}")).unwrap_or_default();
        let name_len = m.name.len() + args.len();
        let pad = 28usize.saturating_sub(name_len);
        rows.push(Some(i));
        lines.push(Line::from(vec![
            Span::styled(
                if selected { "❯ " } else { "  " },
                if selected {
                    Style::default().fg(app.theme.accent)
                } else {
                    Style::default().fg(app.theme.muted)
                },
            ),
            Span::styled(
                m.name,
                if selected {
                    Style::default().fg(app.theme.accent).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(app.theme.text)
                },
            ),
            Span::styled(args, Style::default().fg(app.theme.positive)),
            Span::styled(" ".repeat(pad), Style::default().fg(app.theme.muted)),
            Span::styled(
                m.description,
                if selected {
                    Style::default().fg(app.theme.text)
                } else {
                    Style::default().fg(app.theme.muted)
                },
            ),
        ]));
    }

    let (start, content, above, below) =
        suggestion_window(sel_row, lines.len(), &rows, MAX_SUGGESTIONS);

    let mut visible: Vec<Line> = Vec::new();
    if above > 0 {
        app.palette_rows.push(None);
        visible.push(Line::from(Span::styled(
            format!("  {above} above"),
            Style::default().fg(app.theme.accent),
        )));
    }
    for (i, line) in lines[start..start + content].iter().enumerate() {
        app.palette_rows.push(rows[start + i]);
        visible.push(line.clone());
    }
    if below > 0 {
        app.palette_rows.push(None);
        visible.push(Line::from(Span::styled(
            format!("  + {below} more"),
            Style::default().fg(app.theme.accent),
        )));
    }

    f.render_widget(Paragraph::new(visible).wrap(Wrap { trim: false }), area);
}

fn render_chips(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let mut x = area.x;
    let label_widths = ["Refresh", "Export", "Quit"].map(|s| s.len() as u16);
    for (i, chip) in app.chips.iter_mut().enumerate() {
        let w = label_widths[i] + 4;
        let rect = Rect { x, y: area.y, width: w, height: 1 };
        chip.rect = rect;
        let style = if chip.hovered {
            Style::default().fg(app.theme.selection_fg).bg(app.theme.hover_bg)
        } else {
            Style::default().fg(app.theme.accent)
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(format!(" [{}] ", chip.label), style))),
            rect,
        );
        x += w;
    }
}

fn render_overlay(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let Some(overlay) = &app.overlay else {
        app.overlay_rect = None;
        return;
    };
    let msg = match overlay {
        OverlayKind::Export { message } | OverlayKind::Shell { message } => message,
    };
    let w = area.width.clamp(30, 60);
    let popup = Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + area.height / 2 - 2,
        width: w,
        height: 4,
    };
    app.overlay_rect = Some(popup);
    f.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" tokentracker ")
        .style(Style::default().bg(app.theme.overlay_bg));
    let para = Paragraph::new(Line::from(Span::styled(
        msg.clone(),
        Style::default().fg(app.theme.accent),
    )))
    .block(block)
    .alignment(ratatui::layout::Alignment::Center)
    .wrap(Wrap { trim: true });
    f.render_widget(para, popup);
}

pub fn run(db: &Database, days: u32) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    execute!(stdout, crossterm::event::EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(db, days)?;

    let res = (|| -> Result<()> {
        let mut area = Rect::default();
        while !app.quit {
            terminal.draw(|f| {
                area = f.area();
                // Layout matching vitorrent: title -> tabs -> [spacer] -> chips -> hint ->
                // divider -> body (flexible, only element that grows) ->
                // suggestions (when palette open) -> command bar (always last).
                let mut constraints = vec![
                    Constraint::Length(1),          // 0 title
                    Constraint::Length(1),          // 1 tabs
                    Constraint::Length(1),          // 2 spacer (between tabs and chips)
                    Constraint::Length(1),          // 3 chips
                    Constraint::Length(1),          // 4 hint
                    Constraint::Length(1),          // 5 divider (horizontal rule)
                    Constraint::Min(1),             // 6 body (flexible)
                ];
                if app.palette_open {
                    constraints.push(Constraint::Length(MAX_SUGGESTIONS as u16)); // 7 suggestions
                }
                constraints.push(Constraint::Length(1)); // command bar (always last)
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints(constraints)
                    .split(area);

                render_title(f, &app, chunks[0]);
                render_tabs(f, &mut app, chunks[1]);
                render_chips(f, &mut app, chunks[3]);

                let mut state = app.list_state.clone();
                match app.view {
                    View::Summary => render_summary(f, &app.summary, &app.theme, chunks[6]),
                    View::Daily => render_period_rows(f, &app.daily, "Daily Usage", &app.theme, &mut state, chunks[6]),
                    View::Weekly => render_period_rows(f, &app.weekly, "Weekly Usage", &app.theme, &mut state, chunks[6]),
                    View::Monthly => render_period_rows(f, &app.monthly, "Monthly Usage", &app.theme, &mut state, chunks[6]),
                    View::Recent => render_recent(f, &app.recent, &app.theme, &mut state, chunks[6]),
                    View::Models => render_models(f, &app.models, &app.theme, &mut state, chunks[6]),
                    View::Status => render_status(f, &app.status, &app.theme, &mut state, chunks[6]),
                }
                app.list_state = state;

                let cmdbar_idx = if app.palette_open { 8 } else { 7 };

                if app.palette_open {
                    render_suggestions(f, &mut app, chunks[7]);
                }
                render_command_bar(f, &app, chunks[cmdbar_idx]);

                let hint = Line::from(Span::styled(
                    " [Tab] next  [1-7] jump  [↑/↓] scroll  [PgUp/PgDn] page  [/] commands  [q] quit  [r] refresh  [mouse] click/scroll ",
                    Style::default().fg(app.theme.muted),
                ));
                f.render_widget(
                    Paragraph::new(hint).alignment(ratatui::layout::Alignment::Center),
                    chunks[4],
                );
                // Draw a horizontal divider rule in the divider chunk
                let divider_line = Line::from(Span::raw("─".repeat(area.width.into())));
                f.render_widget(
                    Paragraph::new(divider_line).alignment(ratatui::layout::Alignment::Center),
                    chunks[5],
                );

                render_overlay(f, &mut app, area);
            })?;

            if event::poll(std::time::Duration::from_millis(200))? {
                let ev = event::read()?;
                match ev {
                    Event::Key(key) => {
                        if key.kind != KeyEventKind::Press {
                            continue;
                        }
                        if app.palette_open {
                            match key.code {
                                KeyCode::Esc => {
                                    app.palette_open = false;
                                    app.command.clear();
                                }
                                KeyCode::Enter => {
                                    if let Some(cmd) = app.commit_palette_selection() {
                                        app.execute_command(db, &cmd);
                                    } else {
                                        // Fully-typed command + arg (e.g. "/days 365"):
                                        // suggestion list is empty, so check the raw input.
                                        let trimmed = app.command.trim().to_string();
                                        if is_fully_typed_command(&trimmed) {
                                            app.execute_command(db, &trimmed);
                                        }
                                    }
                                }
                                KeyCode::Tab => {
                                    let matches = match_commands(&app.command);
                                    if !matches.is_empty() {
                                        app.suggestion_index =
                                            (app.suggestion_index + 1) % matches.len();
                                    }
                                }
                                KeyCode::Up => {
                                    let matches = match_commands(&app.command);
                                    if !matches.is_empty() {
                                        app.suggestion_index =
                                            app.suggestion_index.saturating_sub(1);
                                    }
                                }
                                KeyCode::Down => {
                                    let matches = match_commands(&app.command);
                                    if !matches.is_empty() {
                                        app.suggestion_index =
                                            (app.suggestion_index + 1).min(matches.len() - 1);
                                    }
                                }
                                KeyCode::Backspace => {
                                    app.command.pop();
                                }
                                KeyCode::Char(c) => app.command.push(c),
                                _ => {}
                            }
                            continue;
                        }
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => {
                                if app.overlay.is_some() {
                                    app.overlay = None;
                                } else {
                                    app.quit = true;
                                }
                            }
                            KeyCode::Char('/') => {
                                app.palette_open = true;
                                app.command = "/".to_string();
                                app.suggestion_index = 0;
                            }
                            KeyCode::Char('r') => {
                                app.reload(db);
                                app.status_msg = "Refreshed".to_string();
                            }
                            KeyCode::Tab => {
                                let idx = app.view.number() as usize % View::ALL.len();
                                app.set_view(View::ALL[idx]);
                            }
                            KeyCode::Char(c @ '1'..='7') => {
                                let v = View::ALL[(c as u8 - b'1') as usize];
                                app.set_view(v);
                            }
                            KeyCode::Up => app.move_selection(-1),
                            KeyCode::Down => app.move_selection(1),
                            KeyCode::PageUp => app.move_selection(-15),
                            KeyCode::PageDown => app.move_selection(15),
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                app.quit = true
                            }
                            _ => {}
                        }
                    }
                    Event::Mouse(m) => {
                        match m.kind {
                            MouseEventKind::ScrollUp => {
                                if app.palette_open {
                                    let matches = match_commands(&app.command);
                                    if !matches.is_empty() {
                                        app.suggestion_index =
                                            app.suggestion_index.saturating_sub(1);
                                    }
                                } else {
                                    app.move_selection(-3);
                                }
                            }
                            MouseEventKind::ScrollDown => {
                                if app.palette_open {
                                    let matches = match_commands(&app.command);
                                    if !matches.is_empty() {
                                        app.suggestion_index =
                                            (app.suggestion_index + 1).min(matches.len() - 1);
                                    }
                                } else {
                                    app.move_selection(3);
                                }
                            }
                            MouseEventKind::Down(MouseButton::Left) => {
                                let y = m.row;
                                let x = m.column;
                                // Click a tab.
                                if !app.palette_open && y == area.y + 1 {
                                    if let Some(i) = app
                                        .tabs
                                        .iter()
                                        .position(|t| t.rect.contains((x, y).into()))
                                    {
                                        app.set_view(View::ALL[i]);
                                    }
                                    continue;
                                }
                                // Click a bottom chip.
                                if !app.palette_open && app.overlay.is_none() {
                                    let clicked = app
                                        .chips
                                        .iter()
                                        .find(|c| c.rect.contains((x, y).into()))
                                        .map(|c| c.label);
                                    match clicked {
                                        Some("Refresh") => {
                                            app.reload(db);
                                            app.status_msg = "Refreshed".to_string();
                                        }
                                        Some("Export") => {
                                            app.command = "/export ".to_string();
                                            app.palette_open = true;
                                            app.suggestion_index = 0;
                                        }
                                        Some("Quit") => app.quit = true,
                                        _ => {}
                                    }
                                }
                                // Click inside an overlay popup dismisses it.
                                if app.overlay.is_some() {
                                    if let Some(popup) = app.overlay_rect {
                                        if popup.contains((x, y).into()) {
                                            app.overlay = None;
                                            continue;
                                        }
                                    }
                                }
                                // Click a palette suggestion.
                                if app.palette_open {
                                    if let Some(popup) = app.palette_rect {
                                        if popup.contains((x, y).into()) {
                                            let rel = (y - popup.y) as usize;
                                            if let Some(Some(idx)) =
                                                app.palette_rows.get(rel)
                                            {
                                                app.suggestion_index = *idx;
                                                if let Some(cmd) =
                                                    app.commit_palette_selection()
                                                {
                                                    app.execute_command(db, &cmd);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            MouseEventKind::Moved => {
                                let pos = (m.column, m.row).into();
                                if app.palette_open || app.overlay.is_some() {
                                    for chip in &mut app.chips {
                                        chip.hovered = false;
                                    }
                                    for tab in &mut app.tabs {
                                        tab.hovered = false;
                                    }
                                } else {
                                    for chip in &mut app.chips {
                                        chip.hovered = chip.rect.contains(pos);
                                    }
                                    for tab in &mut app.tabs {
                                        tab.hovered = tab.rect.contains(pos);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    Event::Resize(_, _) => {}
                    _ => {}
                }
            }
        }
        Ok(())
    })();

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    execute!(terminal.backend_mut(), crossterm::event::DisableMouseCapture)?;
    terminal.show_cursor()?;
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_commands_lists_all_on_bare_slash() {
        assert_eq!(match_commands("/").len(), COMMANDS.len());
    }

    #[test]
    fn match_commands_prefix_filters() {
        let hits = match_commands("/day");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "/days");
    }

    #[test]
    fn match_commands_empty_on_full_command_with_arg() {
        assert!(match_commands("/days 365").is_empty());
    }

    #[test]
    fn match_commands_empty_on_non_command() {
        assert!(match_commands("hello").is_empty());
    }

    #[test]
    fn fully_typed_command_detects_days() {
        assert!(is_fully_typed_command("/days 365"));
        assert!(!is_fully_typed_command("/days "));
        assert!(!is_fully_typed_command("/days"));
        assert!(!is_fully_typed_command("/"));
    }

    #[test]
    fn commit_palette_selection_submits_bare_command_with_args_on_second_enter() {
        let mut app = test_app();
        // Simulate user typing "/the" (prefix) and pressing Enter -> autocompletes to "/theme "
        app.command = "/the".to_string();
        app.palette_open = true;
        app.suggestion_index = 0; // /theme is first match for "/the"
        
        // First Enter: should autocomplete to "/theme " and stay in palette
        let result1 = app.commit_palette_selection();
        assert_eq!(result1, None, "first Enter should autocomplete and stay");
        assert_eq!(app.command, "/theme ", "command should be autocompleted with space");
        
        // Now user presses Enter again without typing an argument
        // Should submit "/theme" (bare command) to execute_command which shows available themes
        let result2 = app.commit_palette_selection();
        assert_eq!(result2, Some("/theme".to_string()), "second Enter should submit bare command");
    }

    #[test]
    fn commit_palette_selection_works_for_days_and_export_too() {
        let mut app = test_app();
        
        // Test /days (use "/day" prefix to avoid matching "/daily" first)
        app.command = "/day".to_string();
        app.palette_open = true;
        app.suggestion_index = 0;
        let _ = app.commit_palette_selection(); // autocompletes to "/days "
        let result = app.commit_palette_selection(); // second Enter submits bare
        assert_eq!(result, Some("/days".to_string()), "/days should submit on second Enter");
        
        // Test /export (use "/exp" prefix)
        let mut app2 = test_app();
        app2.command = "/exp".to_string();
        app2.palette_open = true;
        app2.suggestion_index = 0;
        let _ = app2.commit_palette_selection(); // autocompletes to "/export "
        let result2 = app2.commit_palette_selection(); // second Enter submits bare
        assert_eq!(result2, Some("/export".to_string()), "/export should submit on second Enter");
    }

    #[test]
    fn suggestion_window_all_rows_fit() {
        // 5 commands, no headers: fits in 6 rows, nothing hidden.
        let rows: Vec<Option<usize>> = (0..5).map(Some).collect();
        let (start, content, above, below) = suggestion_window(0, rows.len(), &rows, 6);
        assert_eq!((start, content, above, below), (0, 5, 0, 0));
    }

    #[test]
    fn suggestion_window_selection_stays_visible_at_end() {
        // 12 commands in 6-row window; selecting the last must scroll it in.
        let rows: Vec<Option<usize>> = (0..12).map(Some).collect();
        let (start, content, _above, below) = suggestion_window(11, rows.len(), &rows, 6);
        assert!(start <= 11 && 11 < start + content);
        assert_eq!(below, 0, "end-of-list should have nothing hidden below");
    }

    #[test]
    fn suggestion_window_headers_do_not_count_as_hidden() {
        // 3 categories x 4 commands, headers interleaved; select the last row.
        let mut rows: Vec<Option<usize>> = Vec::new();
        let mut idx = 0;
        for _cat in 0..3 {
            rows.push(None);
            for _ in 0..4 {
                rows.push(Some(idx));
                idx += 1;
            }
        }
        let total = rows.len();
        let (start, content, _above, below) = suggestion_window(total - 1, total, &rows, 6);
        assert!(start + content > total - 1, "selection stays in the window");
        // Headers above/below are not hidden commands.
        assert_eq!(below, 0);
    }

    #[test]
    fn suggestion_window_reports_hidden_above_and_below() {
        // 20 flat commands, window 6, selection in the middle.
        let rows: Vec<Option<usize>> = (0..20).map(Some).collect();
        let (start, content, above, below) = suggestion_window(10, rows.len(), &rows, 6);
        assert!(start <= 10 && 10 < start + content);
        assert!(above > 0, "10 commands above a centered window");
        assert!(below > 0, "commands below the window");
        assert_eq!(above + content + below, 20);
    }

    #[test]
    fn suggestions_render_windowed_rows_match_clicks() {
        // Build an App state as render_suggestions expects, then verify the
        // recorded palette_rows reflect a windowed slice of the commands.
        let mut app = test_app();
        app.palette_open = true;
        app.command = "/".to_string();
        app.suggestion_index = COMMANDS.len() - 1;

        let area = Rect::new(0, 0, 80, 6);
        let backend = ratatui::backend::TestBackend::new(80, 6);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_suggestions(f, &mut app, area))
            .unwrap();

        // Last command must be represented in the visible rows.
        assert!(app.palette_rows.iter().any(|r| *r == Some(COMMANDS.len() - 1)));
        // Selected index is not null anywhere except headers/markers.
        assert_eq!(app.palette_rect, Some(area));
    }

    #[test]
    fn suggestions_click_row_selects_windowed_command() {
        let mut app = test_app();
        app.palette_open = true;
        app.command = "/".to_string();
        app.suggestion_index = 0;

        let area = Rect::new(0, 0, 80, 6);
        let backend = ratatui::backend::TestBackend::new(80, 6);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_suggestions(f, &mut app, area))
            .unwrap();

        // Row 0 is either a header or the first command; clicking it must map
        // to a valid command index via palette_rows.
        let first_some = app
            .palette_rows
            .iter()
            .enumerate()
            .find(|(_, r)| r.is_some())
            .map(|(i, _)| i)
            .unwrap();
        assert!(app.palette_rows[first_some].is_some());
    }

    #[test]
    fn range_label_matches_each_views_window() {
        assert_eq!(range_label(View::Summary, 30), Some("last 30 days".to_string()));
        assert_eq!(range_label(View::Daily, 30), Some("last 30 days".to_string()));
        assert_eq!(
            range_label(View::Weekly, 30),
            Some(format!("last {WEEKLY_WINDOW_WEEKS} weeks"))
        );
        assert_eq!(
            range_label(View::Monthly, 30),
            Some(format!("last {MONTHLY_WINDOW_MONTHS} months"))
        );
        assert_eq!(
            range_label(View::Recent, 30),
            Some(format!("last {RECENT_RECORD_LIMIT} records"))
        );
        assert_eq!(range_label(View::Models, 30), None);
        assert_eq!(range_label(View::Status, 30), None);
    }

    fn test_app() -> App {
        App {
            days: 30,
            summary: Vec::new(),
            daily: Vec::new(),
            weekly: Vec::new(),
            monthly: Vec::new(),
            recent: Vec::new(),
            models: Vec::new(),
            status: Vec::new(),
            view: View::Summary,
            list_state: ListState::default(),
            command: String::new(),
            palette_open: false,
            suggestion_index: 0,
            quit: false,
            status_msg: String::new(),
            chips: [Chip::default(), Chip::default(), Chip::default()],
            tabs: [Chip::default(); 7],
            overlay: None,
            palette_rect: None,
            palette_rows: Vec::new(),
            overlay_rect: None,
            theme: Theme::default(),
        }
    }
}