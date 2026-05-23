//! Right pane: the agent's stated intent for the file under the cursor, with the
//! session title/last-prompt as a header and a confidence indicator. The "why"
//! beside the "what" is the tool's headline feature.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};

use crate::app::AppState;
use crate::tui::theme;

/// Compact-mode cap on intent text before `i` expands it.
const COMPACT_CHARS: usize = 280;

pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let block = Block::bordered().title(" Intent ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines = Vec::new();
    lines.extend(header(state));
    lines.push(Line::from(""));
    lines.extend(body(state));

    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        inner,
    );
}

fn header(state: &AppState) -> Vec<Line<'static>> {
    let Some(session) = &state.session else {
        return vec![Line::styled(
            "no agent session",
            Style::default().fg(theme::GUTTER_FG),
        )];
    };

    let mut lines = Vec::new();
    let title = session
        .title
        .clone()
        .or_else(|| session.last_prompt.clone())
        .unwrap_or_else(|| "(untitled session)".to_string());
    lines.push(Line::styled(
        title,
        Style::default().add_modifier(Modifier::BOLD),
    ));
    lines.push(Line::styled(
        session.base_label.clone(),
        Style::default().fg(theme::HUNK_HEADER_FG),
    ));
    lines
}

fn body(state: &AppState) -> Vec<Line<'static>> {
    // The file under the cursor, for the "intent for which file" framing.
    let file = state
        .current_row()
        .and_then(|r| state.diff.files.get(r.file()))
        .map(|f| f.path.display().to_string());

    let Some(intent) = state.current_intent() else {
        let mut lines = Vec::new();
        if let Some(path) = file {
            lines.push(Line::styled(path, Style::default().fg(theme::GUTTER_FG)));
        }
        lines.push(Line::styled(
            "(no recorded intent for this file)",
            Style::default().fg(theme::GUTTER_FG),
        ));
        return lines;
    };

    let mut lines = Vec::new();
    if let Some(path) = file {
        lines.push(Line::styled(
            path,
            Style::default().fg(theme::GUTTER_FG),
        ));
    }
    lines.push(why_line(intent.confidence));
    lines.push(Line::from(""));

    let text = &intent.text;
    if !state.intent_detail && text.chars().count() > COMPACT_CHARS {
        let truncated: String = text.chars().take(COMPACT_CHARS).collect();
        lines.push(Line::raw(format!("{truncated}…")));
        lines.push(Line::from(""));
        lines.push(Line::styled(
            "i: show full intent",
            Style::default().fg(theme::GUTTER_FG),
        ));
    } else {
        lines.push(Line::raw(text.clone()));
    }
    lines
}

/// A "WHY" label with a confidence meter.
fn why_line(confidence: f32) -> Line<'static> {
    let filled = (confidence.clamp(0.0, 1.0) * 5.0).round() as usize;
    let meter: String = "●".repeat(filled) + &"○".repeat(5 - filled);
    let pct = (confidence.clamp(0.0, 1.0) * 100.0).round() as u32;
    Line::from(vec![
        Span::styled(
            "WHY ",
            Style::default()
                .fg(theme::APPROVED_FG)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(meter, Style::default().fg(theme::APPROVED_FG)),
        Span::styled(format!(" {pct}%"), Style::default().fg(theme::GUTTER_FG)),
    ])
}
