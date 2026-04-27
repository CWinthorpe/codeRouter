use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

pub fn render(frame: &mut Frame, area: Rect, proxy_running: bool) {
    let (indicator, color) = if proxy_running {
        ("● Proxy Running", Color::Green)
    } else {
        ("● Proxy Stopped", Color::Red)
    };

    let left = Span::styled(
        indicator,
        Style::default()
            .fg(color)
            .add_modifier(Modifier::BOLD),
    );
    let right = Span::styled(
        " q:Quit  ?:Help  Tab:Switch ",
        Style::default().fg(Color::DarkGray),
    );

    let line = Line::from(vec![left, Span::raw("  "), right]);
    let bar = Paragraph::new(line).style(Style::default().bg(Color::Black));
    frame.render_widget(bar, area);
}
