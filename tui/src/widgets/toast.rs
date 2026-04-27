use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

pub fn render(frame: &mut Frame, _area: Rect, text: &str) {
    let popup = Paragraph::new(text)
        .style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .alignment(Alignment::Center);

    let height = 3u16;
    let width = (text.len() as u16 + 4).min(frame.area().width);
    let x = (frame.area().width.saturating_sub(width)) / 2;
    let y = frame.area().height.saturating_sub(height) - 2;

    let area = Rect::new(x, y, width, height);
    frame.render_widget(popup, area);
}
