use crate::app::TAB_NAMES;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

pub fn render(frame: &mut Frame, area: Rect, active_tab: usize) {
    let mut spans: Vec<Span> = Vec::new();
    for (i, name) in TAB_NAMES.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" │ "));
        }
        let style = if i == active_tab {
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(Color::Black)
                .bg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        spans.push(Span::styled(format!(" {} ", name), style));
    }

    let title = Span::styled(
        format!(" CodeRouter v{} ", env!("CARGO_PKG_VERSION")),
        Style::default()
            .add_modifier(Modifier::BOLD)
            .fg(Color::Yellow),
    );

    let mut all_spans = vec![title, Span::raw("  ")];
    all_spans.extend(spans);

    let line = Line::from(all_spans);
    let bar = Paragraph::new(line).style(Style::default().bg(Color::Black));
    frame.render_widget(bar, area);
}
