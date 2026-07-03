//! "Did it actually work?" — the shell commands the agent ran during the run
//! and whether they passed. A compact badge ([`summary_line`]) sits in the
//! Intent panel header; the full list is a centered overlay toggled with `v`.
//!
//! This is the read-only twin of intent: intent is what the agent *said*, this
//! is what it *ran to check itself*. All of it is advisory (see
//! `session::commands`).

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::app::AppState;
use crate::domain::session::{CommandKind, CommandOutcome, CommandRun};
use crate::tui::theme;

/// Verification kinds shown in the compact badge, in display order.
const BADGE_KINDS: [CommandKind; 4] = [
    CommandKind::Test,
    CommandKind::Build,
    CommandKind::Lint,
    CommandKind::Format,
];

/// Max excerpt lines shown under a failed command in the overlay.
const EXCERPT_LINES: usize = 6;

/// A one-line summary of the run's verification work for the Intent header:
/// the latest of each verification kind, colored by outcome. `None` when the
/// run ran no shell commands at all. `stale` marks evidence that predates the
/// run's last edit — a ✓ that proves nothing about the state under review.
pub fn summary_line(commands: &[CommandRun], stale: bool) -> Option<Line<'static>> {
    if commands.is_empty() {
        return None;
    }

    let mut spans = vec![Span::styled(
        "VERIFY ",
        Style::default()
            .fg(theme::hunk_header_fg())
            .add_modifier(Modifier::BOLD),
    )];

    let mut shown = false;
    for kind in BADGE_KINDS {
        // Latest command of this kind wins (it reflects the run's final state).
        if let Some(cmd) = commands.iter().rev().find(|c| c.kind == kind) {
            spans.push(Span::styled(
                format!("{} {}  ", icon(cmd.outcome), kind.label()),
                Style::default().fg(outcome_fg(cmd.outcome)),
            ));
            shown = true;
        }
    }

    // Commands ran, but none were recognizable verification work.
    if !shown {
        spans.push(Span::styled(
            format!("ran {} command(s)", commands.len()),
            Style::default().fg(theme::gutter_fg()),
        ));
    } else if stale {
        spans.push(Span::styled(
            "⟳ stale (edits after last check)",
            Style::default()
                .fg(theme::needs_attention_fg())
                .add_modifier(Modifier::BOLD),
        ));
    }
    Some(Line::from(spans))
}

/// The full verification overlay for the loaded run.
pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let commands = state.session.as_ref().map(|s| s.commands.as_slice());
    let run_label = state
        .session
        .as_ref()
        .map(|s| s.base_label.clone())
        .unwrap_or_default();

    let mut lines: Vec<Line> = Vec::new();
    match commands {
        Some(cmds) if !cmds.is_empty() => {
            for cmd in cmds {
                lines.push(command_line(cmd));
                if cmd.outcome == CommandOutcome::Failed {
                    lines.extend(excerpt_lines(&cmd.output_excerpt));
                }
            }
            lines.push(Line::from(""));
            lines.push(Line::styled(tally(cmds), Style::default().fg(theme::gutter_fg())));
            if state.session.as_ref().is_some_and(|s| s.verify_stale) {
                lines.push(Line::styled(
                    "⟳ the last verification ran before the run's final edits",
                    Style::default()
                        .fg(theme::needs_attention_fg())
                        .add_modifier(Modifier::BOLD),
                ));
            }
        }
        _ => lines.push(Line::styled(
            "The agent ran no shell commands during this run.",
            Style::default().fg(theme::gutter_fg()),
        )),
    }

    let title = if run_label.is_empty() {
        " Verification ".to_string()
    } else {
        format!(" Verification — {run_label} ")
    };

    let width = 76.min(area.width);
    let height = (lines.len() as u16 + 2).min(area.height);
    let popup = centered(area, width, height);
    frame.render_widget(Clear, popup);
    let block = Block::bordered()
        .title(title)
        .style(Style::default().bg(theme::bg()).fg(theme::fg()));
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

/// `<icon> <kind>  <first line of the command>`.
fn command_line(cmd: &CommandRun) -> Line<'static> {
    let head = cmd.command.lines().next().unwrap_or("").trim();
    let head: String = head.chars().take(60).collect();
    Line::from(vec![
        Span::styled(
            format!("{} {:<5} ", icon(cmd.outcome), cmd.kind.label()),
            Style::default()
                .fg(outcome_fg(cmd.outcome))
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(head),
    ])
}

/// Indented, dimmed tail of a failed command's output.
fn excerpt_lines(excerpt: &str) -> Vec<Line<'static>> {
    excerpt
        .lines()
        .take(EXCERPT_LINES)
        .map(|l| {
            Line::from(vec![
                Span::raw("        │ "),
                Span::styled(l.to_string(), Style::default().fg(theme::gutter_fg())),
            ])
        })
        .collect()
}

fn tally(cmds: &[CommandRun]) -> String {
    let ok = cmds.iter().filter(|c| c.outcome == CommandOutcome::Ok).count();
    let failed = cmds
        .iter()
        .filter(|c| c.outcome == CommandOutcome::Failed)
        .count();
    let unknown = cmds.len() - ok - failed;
    let mut parts = vec![format!("{} command(s)", cmds.len()), format!("{ok} ok")];
    if failed > 0 {
        parts.push(format!("{failed} failed"));
    }
    if unknown > 0 {
        parts.push(format!("{unknown} pending"));
    }
    parts.join(" · ")
}

fn icon(outcome: CommandOutcome) -> &'static str {
    match outcome {
        CommandOutcome::Ok => "✓",
        CommandOutcome::Failed => "✗",
        CommandOutcome::Unknown => "·",
    }
}

fn outcome_fg(outcome: CommandOutcome) -> ratatui::style::Color {
    match outcome {
        CommandOutcome::Ok => theme::added_fg(),
        CommandOutcome::Failed => theme::removed_fg(),
        CommandOutcome::Unknown => theme::gutter_fg(),
    }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [horizontal] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(area);
    let [centered] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(horizontal);
    centered
}
