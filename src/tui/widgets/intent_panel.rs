//! Right pane: the agent's stated intent for the file under the cursor, with the
//! session title/last-prompt as a header and a confidence indicator. The "why"
//! beside the "what" is the tool's headline feature.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};

use crate::app::AppState;
use crate::app::state::IntentScope;
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
    lines.extend(note_section(state));

    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        inner,
    );
}

/// The reviewer's own note for the current hunk, when one exists.
fn note_section(state: &AppState) -> Vec<Line<'static>> {
    let Some(href) = state.current_hunk_ref() else {
        return Vec::new();
    };
    let Some(note) = state.review.notes.get(&href) else {
        return Vec::new();
    };
    let mut lines = vec![
        Line::from(""),
        Line::styled(
            "NOTE",
            Style::default()
                .fg(theme::needs_attention_fg())
                .add_modifier(Modifier::BOLD),
        ),
    ];
    lines.extend(note.lines().map(|l| Line::raw(l.to_string())));
    lines
}

fn header(state: &AppState) -> Vec<Line<'static>> {
    let Some(session) = &state.session else {
        return vec![Line::styled(
            "no agent session",
            Style::default().fg(theme::gutter_fg()),
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

    let mut base = vec![
        Span::styled(
            format!("{}  ", session.provider.label()),
            Style::default().fg(theme::fg()).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            session.base_label.clone(),
            Style::default().fg(theme::hunk_header_fg()),
        ),
    ];
    if session.live {
        base.push(Span::raw("  "));
        base.push(Span::styled(
            "● live",
            Style::default()
                .fg(theme::removed_fg())
                .add_modifier(Modifier::BOLD),
        ));
    }
    lines.push(Line::from(base));

    // What the agent ran to verify itself, if anything — the read-only twin of
    // the per-file intent below.
    if let Some(verify) = super::verification::summary_line(&session.commands) {
        lines.push(verify);
    }
    lines
}

fn body(state: &AppState) -> Vec<Line<'static>> {
    // The file under the cursor, for the "intent for which file" framing.
    let file = state
        .current_row()
        .and_then(|r| state.diff.files.get(r.file()))
        .map(|f| f.path.display().to_string());

    let Some((intent, scope)) = state.current_intent() else {
        let mut lines = Vec::new();
        if let Some(path) = file {
            lines.push(Line::styled(path, Style::default().fg(theme::gutter_fg())));
        }
        lines.push(Line::styled(
            "(no recorded intent for this file)",
            Style::default().fg(theme::gutter_fg()),
        ));
        return lines;
    };

    let mut lines = Vec::new();
    if let Some(path) = file {
        lines.push(Line::styled(
            path,
            Style::default().fg(theme::gutter_fg()),
        ));
    }
    lines.push(why_line(intent.confidence, scope));
    lines.push(Line::from(""));

    let text = &intent.text;
    if !state.intent_detail && text.chars().count() > COMPACT_CHARS {
        let truncated: String = text.chars().take(COMPACT_CHARS).collect();
        lines.push(Line::raw(format!("{truncated}…")));
        lines.push(Line::from(""));
        lines.push(Line::styled(
            "i: show full intent",
            Style::default().fg(theme::gutter_fg()),
        ));
    } else {
        lines.push(Line::raw(text.clone()));
    }
    lines
}

/// A "WHY" label with a confidence meter and the anchoring granularity, so the
/// reviewer knows whether this reasoning drove *this hunk* or just the file.
fn why_line(confidence: f32, scope: IntentScope) -> Line<'static> {
    let filled = (confidence.clamp(0.0, 1.0) * 5.0).round() as usize;
    let meter: String = "●".repeat(filled) + &"○".repeat(5 - filled);
    let pct = (confidence.clamp(0.0, 1.0) * 100.0).round() as u32;
    let scope_label = match scope {
        IntentScope::Hunk => " · this hunk",
        IntentScope::File => " · whole file",
    };
    Line::from(vec![
        Span::styled(
            "WHY ",
            Style::default()
                .fg(theme::intent_fg())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(meter, Style::default().fg(theme::intent_fg())),
        Span::styled(format!(" {pct}%"), Style::default().fg(theme::gutter_fg())),
        Span::styled(scope_label, Style::default().fg(theme::gutter_fg())),
    ])
}
