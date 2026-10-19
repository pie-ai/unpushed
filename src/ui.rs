//! Rendering the application state with ratatui.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Row as TableRow, Table, Wrap};
use ratatui::Frame;

use crate::app::{Activity, App, Level, Mode, Row, View};
use crate::model::{Divergence, MainState, ScanOutcome, UpstreamState};

const HELP: &str = "j/k select · Space mark · p push · A visible · P marked · \
                    g gitui · f/F fetch · r/R refresh · / filter · a all · q quit";

pub fn draw(frame: &mut Frame, app: &App) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .areas(frame.area());

    draw_header(frame, app, header);
    draw_table(frame, app, body);
    draw_footer(frame, app, footer);

    if let Mode::Confirm(confirm) = &app.mode {
        draw_confirm(frame, confirm, frame.area());
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
        .map(|(position, &index)| render_row(&app.rows[index], position == app.cursor))
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

fn render_row(row: &Row, selected: bool) -> TableRow<'static> {
    let mark = if row.marked { "x" } else { " " };

    let (marker, marker_style) = match (&row.outcome, row.activity) {
        (_, Activity::Scanning) => ("…", Style::default().fg(Color::DarkGray)),
        (_, Activity::Fetching) => ("↻", Style::default().fg(Color::Cyan)),
        (_, Activity::Pushing) => ("→", Style::default().fg(Color::Cyan)),
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

    let base = if selected {
        Style::default()
            .bg(Color::Indexed(238))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    TableRow::new(vec![
        Span::styled(mark.to_string(), base.fg(Color::Green)),
        Span::styled(marker.to_string(), base.patch(marker_style)),
        Span::styled(row.repo.name.clone(), base),
        Span::styled(branch, base.fg(Color::Blue)),
        Span::styled(upstream, base),
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
        HELP,
        Style::default().fg(Color::DarkGray),
    )));

    frame.render_widget(Paragraph::new(lines), area);
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
