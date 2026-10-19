//! Rendering the application state with ratatui.

use std::cell::RefCell;
use std::rc::Rc;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Row as TableRow, Table, Wrap};
use ratatui::Frame;

use crate::app::{Activity, App, Diff, Focus, Item, Level, Menu, Mode, Panel, Row, View};
use crate::commits::LineKind;
use crate::model::{Divergence, MainState, ScanOutcome, UpstreamState};

/// Per-focus, because the list keys are no help while reading a diff.
fn help(app: &App) -> &'static str {
    match app.focus() {
        Focus::List => {
            "j/k select · Enter actions · → commits · p push · g gitui · Space mark · \
             A visible · P marked · f/F fetch · r/R refresh · / filter · a all · q quit"
        }
        Focus::Panel => {
            "j/k move · Space fold · → open · ← back · Enter actions · p push · \
             g gitui · f fetch · r refresh · q quit"
        }
        Focus::Diff => "j/k · PgUp/PgDn · Home/End scroll · ← back · q quit",
    }
}

pub fn draw(frame: &mut Frame, app: &App) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .areas(frame.area());

    draw_header(frame, app, header);

    // Each level gets the room it needs: the list alone, the list beside the
    // commit tree, and a diff over the full width — a diff is unreadable in
    // half a terminal.
    match (&app.diff, &app.panel) {
        (Some(diff), _) => draw_diff(frame, diff, body),
        (None, Some(panel)) => {
            let [left, right] =
                Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)])
                    .areas(body);
            draw_table(frame, app, left);
            draw_panel(frame, app, panel, right);
        }
        (None, None) => draw_table(frame, app, body),
    }

    draw_footer(frame, app, footer);

    match &app.mode {
        Mode::Confirm(confirm) => draw_confirm(frame, confirm, frame.area()),
        Mode::Menu(menu) => draw_menu(frame, app, menu, frame.area()),
        _ => {}
    }
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let summary = app.summary();
    let visible = app.visible().len();

    let mut spans = vec![
        Span::styled(
            format!("{} need work", summary.with_work),
            Style::default().fg(Color::Yellow),
        ),
        Span::raw(" · "),
        Span::raw(format!("{} clean", summary.clean)),
        Span::raw(" · "),
        Span::styled(
            format!("{} failed", summary.failed),
            if summary.failed > 0 {
                Style::default().fg(Color::Red)
            } else {
                Style::default()
            },
        ),
        Span::raw(format!("   ({} repos, {visible} visible)", app.rows.len())),
    ];

    if app.fetching > 0 {
        spans.push(Span::styled(
            format!("   fetching … {} left", app.fetching),
            Style::default().fg(Color::Cyan),
        ));
    }

    let mut lines = vec![Line::from(spans)];

    let mut second = vec![Span::styled(
        app.base.clone(),
        Style::default().fg(Color::DarkGray),
    )];
    if !app.filter.is_empty() {
        second.push(Span::raw("   Filter: "));
        second.push(Span::styled(
            app.filter.clone(),
            Style::default().fg(Color::Magenta),
        ));
    }
    if app.view == View::All {
        second.push(Span::styled(
            "   [all]",
            Style::default().fg(Color::DarkGray),
        ));
    }
    lines.push(Line::from(second));

    frame.render_widget(Paragraph::new(lines), area);
}

fn draw_table(frame: &mut Frame, app: &App, area: Rect) {
    let visible = app.visible();
    if visible.is_empty() {
        let text = if app.rows.is_empty() {
            "No git checkouts found."
        } else {
            "Nothing to do. Press 'a' to show all repositories."
        };
        frame.render_widget(
            Paragraph::new(text).style(Style::default().fg(Color::DarkGray)),
            area,
        );
        return;
    }

    let rows: Vec<TableRow> = visible
        .iter()
        .enumerate()
        .map(|(position, &index)| {
            render_row(&app.rows[index], position == app.cursor, app.spinner())
        })
        .collect();

    // Repository and branch get exactly the width their content needs (within
    // limits), so the remaining space goes to the trailing column, which holds
    // the action results and is the one worth not truncating.
    let name_width = measure(&visible, app, |row| row.repo.name.chars().count(), 16, 34);
    let branch_width = measure(&visible, app, branch_len, 10, 30);

    let table = Table::new(
        rows,
        [
            Constraint::Length(2),            // mark
            Constraint::Length(2),            // status marker
            Constraint::Length(name_width),   // repository
            Constraint::Length(branch_width), // branch
            Constraint::Length(10),           // vs upstream
            Constraint::Length(13),           // vs default branch
            Constraint::Min(12),              // worktree / note
        ],
    )
    .header(
        TableRow::new(["", "", "Repo", "Branch", "Upstream", "Main", ""])
            .style(Style::default().fg(Color::DarkGray)),
    )
    .column_spacing(1);

    frame.render_widget(table, area);
}

/// Width of a column, sized to its widest entry and clamped to a sane range.
fn measure(
    visible: &[usize],
    app: &App,
    width_of: impl Fn(&Row) -> usize,
    min: u16,
    max: u16,
) -> u16 {
    let widest = visible
        .iter()
        .map(|&index| width_of(&app.rows[index]) as u16)
        .max()
        .unwrap_or(min);
    widest.clamp(min, max)
}

fn branch_len(row: &Row) -> usize {
    match &row.outcome {
        Some(ScanOutcome::Ok(status)) => status.head.label().chars().count(),
        _ => 1,
    }
}

fn render_row(row: &Row, selected: bool, spinner: char) -> TableRow<'static> {
    let mark = if row.marked { "x" } else { " " };

    let spinning = spinner.to_string();
    let (marker, marker_style) = match (&row.outcome, row.activity()) {
        (_, Activity::Scanning) => (spinning.as_str(), Style::default().fg(Color::DarkGray)),
        (_, Activity::Fetching) => (spinning.as_str(), Style::default().fg(Color::Cyan)),
        (_, Activity::Pushing) => (spinning.as_str(), Style::default().fg(Color::Cyan)),
        (None, _) => ("…", Style::default().fg(Color::DarkGray)),
        (Some(ScanOutcome::Failed(_)), _) => ("!", Style::default().fg(Color::Red)),
        (Some(ScanOutcome::Ok(status)), _) => {
            let marker = status.marker();
            let style = match marker {
                '↑' => Style::default().fg(Color::Yellow),
                '↓' => Style::default().fg(Color::Blue),
                '⇅' => Style::default().fg(Color::Magenta),
                '⚠' => Style::default().fg(Color::Cyan),
                _ => Style::default().fg(Color::DarkGray),
            };
            (marker_text(marker), style)
        }
    };

    let (branch, upstream, main, trailing, trailing_style) = match &row.outcome {
        None => (
            String::new(),
            String::new(),
            String::new(),
            "reading …".to_string(),
            Style::default().fg(Color::DarkGray),
        ),
        Some(ScanOutcome::Failed(err)) => (
            "—".to_string(),
            "failed".to_string(),
            String::new(),
            err.message.clone(),
            Style::default().fg(Color::Red),
        ),
        Some(ScanOutcome::Ok(status)) => {
            let upstream = match &status.upstream {
                UpstreamState::Tracking { divergence, .. } => divergence_text(*divergence),
                UpstreamState::NotSet => "no upstream".to_string(),
                UpstreamState::Gone { .. } => "upstream gone".to_string(),
            };
            let main = match &status.main {
                MainState::Compared { divergence, .. } => divergence_text(*divergence),
                MainState::OnMain { .. } => "on default".to_string(),
                MainState::Unknown => "?".to_string(),
            };
            // A note from the last action outranks the dirty summary: it is
            // the newer and more actionable piece of information.
            let (trailing, style) = match &row.note {
                Some(note) => (note.text.clone(), level_style(note.level)),
                None if status.worktree.is_dirty() => {
                    (describe_worktree(status), Style::default().fg(Color::Cyan))
                }
                None => (String::new(), Style::default()),
            };
            (status.head.label(), upstream, main, trailing, style)
        }
    };

    // What is happening right now outranks anything that was true before it.
    let (trailing, trailing_style) = match row.activity() {
        Activity::Scanning => ("reading …".to_string(), Style::default().fg(Color::Cyan)),
        Activity::Fetching => ("fetching …".to_string(), Style::default().fg(Color::Cyan)),
        Activity::Pushing => ("pushing …".to_string(), Style::default().fg(Color::Cyan)),
        Activity::Idle => (trailing, trailing_style),
    };

    // Busy rows are dimmed so the eye lands on them as "not yet settled".
    // DarkGray rather than Modifier::DIM: not every terminal renders DIM.
    let busy = row.activity() != Activity::Idle;

    let base = if selected {
        Style::default()
            .bg(Color::Indexed(238))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    let dim = |style: Style| {
        if busy {
            style.fg(Color::DarkGray)
        } else {
            style
        }
    };

    TableRow::new(vec![
        Span::styled(mark.to_string(), base.fg(Color::Green)),
        Span::styled(marker.to_string(), base.patch(marker_style)),
        Span::styled(row.repo.name.clone(), dim(base)),
        Span::styled(branch, dim(base.fg(Color::Blue))),
        Span::styled(upstream, dim(base)),
        Span::styled(main, base.fg(Color::DarkGray)),
        Span::styled(trailing, base.patch(trailing_style)),
    ])
    .style(base)
}

fn marker_text(marker: char) -> &'static str {
    match marker {
        '↑' => "↑",
        '↓' => "↓",
        '⇅' => "⇅",
        '⚠' => "⚠",
        _ => "·",
    }
}

fn level_style(level: Level) -> Style {
    match level {
        Level::Info => Style::default().fg(Color::Green),
        Level::Warn => Style::default().fg(Color::Yellow),
        Level::Error => Style::default().fg(Color::Red),
    }
}

fn divergence_text(divergence: Divergence) -> String {
    match (divergence.ahead, divergence.behind) {
        (0, 0) => "—".into(),
        (ahead, 0) => format!("↑{ahead}"),
        (0, behind) => format!("↓{behind}"),
        (ahead, behind) => format!("↑{ahead} ↓{behind}"),
    }
}

fn describe_worktree(status: &crate::model::RepoStatus) -> String {
    let w = &status.worktree;
    let mut parts = Vec::new();
    if w.staged > 0 {
        parts.push(format!("{} staged", w.staged));
    }
    if w.unstaged > 0 {
        parts.push(format!("{} modified", w.unstaged));
    }
    if w.untracked > 0 {
        parts.push(format!("{} untracked", w.untracked));
    }
    if w.conflicted > 0 {
        parts.push(format!("{} conflicted", w.conflicted));
    }
    format!("⚠ {}", parts.join(", "))
}

/// The commit tree: unpushed commits, with their files under them.
fn draw_panel(frame: &mut Frame, app: &App, panel: &Panel, area: Rect) {
    let name = app
        .rows
        .get(panel.repo)
        .map(|r| r.repo.name.as_str())
        .unwrap_or("?");

    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [title_area, list_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);

    let title = if panel.loading {
        format!(" {name} — reading …")
    } else if let Some(err) = &panel.error {
        format!(" {name} — {err}")
    } else {
        format!(" {name} — {} unpushed", panel.commits.len())
    };
    frame.render_widget(
        Paragraph::new(title).style(Style::default().add_modifier(Modifier::BOLD)),
        title_area,
    );

    if panel.commits.is_empty() {
        // Loading and failure already show in the title; only the genuinely
        // empty case needs a word here.
        let text = if panel.loading || panel.error.is_some() {
            ""
        } else {
            " Nothing unpushed."
        };
        frame.render_widget(
            Paragraph::new(text).style(Style::default().fg(Color::DarkGray)),
            list_area,
        );
        return;
    }

    let items = panel.items();
    let height = list_area.height as usize;
    // Keep the cursor in view without a scrollbar: show the window that
    // contains it.
    let first = panel.cursor.saturating_sub(height.saturating_sub(1));

    let lines: Vec<Line> = items
        .iter()
        .enumerate()
        .skip(first)
        .take(height)
        .map(|(position, item)| {
            let selected = position == panel.cursor;
            let base = if selected {
                Style::default()
                    .bg(Color::Indexed(238))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            match *item {
                Item::Commit(c) => {
                    let commit = &panel.commits[c];
                    let open = panel.expanded.get(c).copied().unwrap_or(false);
                    let marker = if open { "▾" } else { "▸" };
                    let count = if open || commit.files.is_empty() {
                        String::new()
                    } else {
                        format!("  ({} files)", commit.files.len())
                    };
                    Line::from(vec![
                        Span::styled(format!(" {marker} "), base.fg(Color::DarkGray)),
                        Span::styled(format!("{} ", commit.short_id), base.fg(Color::Yellow)),
                        Span::styled(format!("{} ", commit.date), base.fg(Color::DarkGray)),
                        Span::styled(commit.summary.clone(), base),
                        Span::styled(count, base.fg(Color::DarkGray)),
                    ])
                }
                Item::File(c, f) => {
                    let file = &panel.commits[c].files[f];
                    Line::from(vec![
                        Span::styled("     ", base),
                        Span::styled(
                            format!("{} ", file.status.letter()),
                            base.fg(status_colour(file.status)),
                        ),
                        Span::styled(file.path.clone(), base.fg(Color::Blue)),
                    ])
                }
            }
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), list_area);
}

fn status_colour(status: crate::commits::Status) -> Color {
    use crate::commits::Status;
    match status {
        Status::Added => Color::Green,
        Status::Deleted => Color::Red,
        Status::Modified => Color::Yellow,
        Status::Renamed => Color::Magenta,
        Status::Other => Color::DarkGray,
    }
}

/// Highlighting a diff costs real work, and the interface redraws ten times a
/// second. The result is kept for the file currently on screen; switching
/// files replaces it.
fn highlighted(diff: &Diff) -> Rc<Vec<crate::highlight::Highlighted>> {
    /// Title and line count identify what is cached; together they change
    /// whenever the content does.
    type Cached = Option<(String, usize, Rc<Vec<crate::highlight::Highlighted>>)>;

    thread_local! {
        static CACHE: RefCell<Cached> = const { RefCell::new(None) };
    }
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some((title, len, lines)) = cache.as_ref() {
            if title == &diff.title && *len == diff.lines.len() {
                return Rc::clone(lines);
            }
        }
        let lines = std::rc::Rc::new(crate::highlight::diff(&diff.path, &diff.lines));
        *cache = Some((
            diff.title.clone(),
            diff.lines.len(),
            std::rc::Rc::clone(&lines),
        ));
        lines
    })
}

fn draw_diff(frame: &mut Frame, diff: &Diff, area: Rect) {
    let [title_area, body] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);

    frame.render_widget(
        Paragraph::new(diff.title.clone()).style(Style::default().add_modifier(Modifier::BOLD)),
        title_area,
    );

    if diff.loading {
        frame.render_widget(
            Paragraph::new("reading …").style(Style::default().fg(Color::DarkGray)),
            body,
        );
        return;
    }
    if let Some(err) = &diff.error {
        frame.render_widget(
            Paragraph::new(err.clone()).style(Style::default().fg(Color::Red)),
            body,
        );
        return;
    }
    if diff.lines.is_empty() {
        frame.render_widget(
            Paragraph::new("No textual diff — the file may be binary.")
                .style(Style::default().fg(Color::DarkGray)),
            body,
        );
        return;
    }

    let coloured = highlighted(diff);
    let height = body.height as usize;

    let lines: Vec<Line> = diff
        .lines
        .iter()
        .zip(coloured.iter())
        .skip(diff.scroll)
        .take(height)
        .map(|(line, pieces)| {
            // A one-character gutter rather than a coloured marker: the line
            // colour already says added or removed, and `+`/`-` in front of
            // code makes it harder to read against the original.
            let gutter = match line.kind {
                LineKind::Added => "+ ",
                LineKind::Removed => "- ",
                LineKind::Context => "  ",
                LineKind::Header => "",
            };
            let mut spans = vec![Span::styled(
                gutter.to_string(),
                Style::default().fg(Color::DarkGray),
            )];
            for (style, text) in pieces {
                spans.push(Span::styled(text.clone(), *style));
            }
            Line::from(spans)
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), body);
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let mut lines = Vec::new();

    match (&app.mode, &app.message) {
        (Mode::Filter { draft, .. }, _) => lines.push(Line::from(vec![
            Span::styled("Filter: ", Style::default().fg(Color::Magenta)),
            Span::raw(draft.clone()),
            Span::styled("▏", Style::default().fg(Color::Magenta)),
            Span::styled(
                "   Enter to apply · Esc to discard",
                Style::default().fg(Color::DarkGray),
            ),
        ])),
        (_, Some(message)) => lines.push(Line::from(Span::styled(
            message.text.clone(),
            level_style(message.level),
        ))),
        _ => lines.push(Line::from("")),
    }

    lines.push(Line::from(Span::styled(
        help(app),
        Style::default().fg(Color::DarkGray),
    )));

    frame.render_widget(Paragraph::new(lines), area);
}

/// Everything that can be done to one repository, each with its key.
fn draw_menu(frame: &mut Frame, app: &App, menu: &Menu, area: Rect) {
    let entries = app.menu_entries(menu.repo);
    let name = app
        .rows
        .get(menu.repo)
        .map(|r| r.repo.name.as_str())
        .unwrap_or("?");

    // The key column is sized to the widest hint, so the labels line up and
    // the keys read as a column you can learn.
    let key_width = entries
        .iter()
        .map(|e| e.hint.chars().count())
        .max()
        .unwrap_or(1);
    let width = entries
        .iter()
        .map(|e| key_width + e.label.chars().count() + 8)
        .max()
        .unwrap_or(30)
        .max(name.chars().count() + 6) as u16;
    // Two border rows, a blank above and below the entries, and the Esc line.
    let height = entries.len() as u16 + 5;
    let popup = centered(width.min(area.width), height.min(area.height), area);

    let mut lines = vec![Line::from("")];
    for (index, entry) in entries.iter().enumerate() {
        let selected = index == menu.cursor;
        let base = if selected {
            Style::default()
                .bg(Color::Indexed(238))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::styled("  ", base),
            Span::styled(
                format!("{:<key_width$}", entry.hint),
                base.fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ),
            Span::styled("   ", base),
            Span::styled(entry.label.clone(), base),
            Span::styled("  ", base),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Esc  close",
        Style::default().fg(Color::DarkGray),
    )));

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {name} ")),
        ),
        popup,
    );
}

fn draw_confirm(frame: &mut Frame, confirm: &crate::app::Confirm, area: Rect) {
    // The list of targets is what makes `A` and `P` distinguishable, so the
    // dialog grows with it instead of truncating to a bare count.
    let shown = confirm.targets.len().min(12);
    let height = (shown as u16) + 6;
    let width = 64.min(area.width.saturating_sub(4)).max(20);
    let popup = centered(width, height.min(area.height), area);

    let mut lines = vec![
        Line::from(Span::styled(
            confirm.prompt.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    for name in confirm.targets.iter().take(shown) {
        lines.push(Line::from(format!("  {name}")));
    }
    if confirm.targets.len() > shown {
        lines.push(Line::from(Span::styled(
            format!("  … and {} more", confirm.targets.len() - shown),
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  [y] push    [n] cancel",
        Style::default().fg(Color::Yellow),
    )));

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(Block::default().borders(Borders::ALL).title(" Push ")),
        popup,
    );
}

fn centered(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn divergence_text_covers_all_shapes() {
        assert_eq!(divergence_text(Divergence::default()), "—");
        assert_eq!(
            divergence_text(Divergence {
                ahead: 2,
                behind: 0
            }),
            "↑2"
        );
        assert_eq!(
            divergence_text(Divergence {
                ahead: 0,
                behind: 5
            }),
            "↓5"
        );
        assert_eq!(
            divergence_text(Divergence {
                ahead: 2,
                behind: 5
            }),
            "↑2 ↓5"
        );
    }

    #[test]
    fn centered_popup_fits_inside_a_small_area() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 20,
            height: 6,
        };
        let popup = centered(64, 20, area);
        assert!(popup.width <= area.width);
        assert!(popup.height <= area.height);
        assert!(popup.x + popup.width <= area.x + area.width);
    }
}
