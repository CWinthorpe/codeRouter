use crate::app::TAB_NAMES;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

fn help_lines_for_page(tab: usize) -> Vec<(&'static str, &'static str)> {
    let mut lines: Vec<(&'static str, &'static str)> = Vec::new();

    lines.push(("", ""));
    lines.push(("  1-6   ", "Switch tab"));
    lines.push(("  Tab   ", "Next tab"));
    lines.push(("  S-Tab ", "Previous tab"));
    lines.push(("  q     ", "Quit"));
    lines.push(("  ?     ", "Toggle this help"));
    lines.push(("", ""));

    match tab {
        0 => {
            lines.push(("── Dashboard ──", ""));
            lines.push(("  r     ", "Refresh data"));
            lines.push(("  j/k   ", "Scroll requests"));
            lines.push(("  ↑/↓   ", "Scroll requests"));
        }
        1 => {
            lines.push(("── Providers ──", ""));
            lines.push(("  j/k   ", "Navigate list"));
            lines.push(("  a     ", "Add provider"));
            lines.push(("  e     ", "Edit provider"));
            lines.push(("  d     ", "Delete provider"));
            lines.push(("  t     ", "Toggle enable/disable"));
            lines.push(("  T     ", "Test connection"));
            lines.push(("  R     ", "Refresh models"));
            lines.push(("  Enter ", "View detail"));
            lines.push(("", ""));
            lines.push(("  Detail view:", ""));
            lines.push(("  j/k   ", "Scroll models"));
            lines.push(("  Esc   ", "Back to list"));
            lines.push(("", ""));
            lines.push(("  Add/Edit form:", ""));
            lines.push(("  Tab   ", "Next field"));
            lines.push(("  ←/→   ", "Presets (Add)"));
            lines.push(("  Enter ", "Save"));
            lines.push(("  Esc   ", "Cancel"));
        }
        2 => {
            lines.push(("── Groups ──", ""));
            lines.push(("  j/k   ", "Navigate list"));
            lines.push(("  a     ", "Add group"));
            lines.push(("  e     ", "Edit group"));
            lines.push(("  d     ", "Delete group"));
            lines.push(("  Enter ", "View detail"));
            lines.push(("", ""));
            lines.push(("  Detail view:", ""));
            lines.push(("  j/k   ", "Navigate entries"));
            lines.push(("  J/K   ", "Reorder entry"));
            lines.push(("  e     ", "Toggle entry"));
            lines.push(("  d     ", "Delete entry"));
            lines.push(("  a     ", "Add entry"));
            lines.push(("  f     ", "Edit failover"));
            lines.push(("  Esc   ", "Back to list"));
            lines.push(("", ""));
            lines.push(("  Forms:", ""));
            lines.push(("  Tab   ", "Next field"));
            lines.push(("  Space ", "Toggle checkbox"));
            lines.push(("  Enter ", "Save"));
            lines.push(("  Esc   ", "Cancel"));
        }
        3 => {
            lines.push(("── OpenCode ──", ""));
            lines.push(("  j/k   ", "Navigate"));
            lines.push(("  Tab   ", "Switch section"));
            lines.push(("  t     ", "Toggle provider"));
            lines.push(("  c     ", "Set custom path"));
            lines.push(("  s     ", "Save config"));
            lines.push(("  Enter ", "Edit mapping / Detail"));
            lines.push(("", ""));
            lines.push(("  Custom Agents:", ""));
            lines.push(("  a     ", "Add agent"));
            lines.push(("  e     ", "Edit agent"));
            lines.push(("  d     ", "Delete agent"));
            lines.push(("  Enter ", "View detail"));
            lines.push(("", ""));
            lines.push(("  Agent form:", ""));
            lines.push(("  Tab   ", "Next field"));
            lines.push(("  j/k   ", "Change option"));
            lines.push(("  Space ", "Toggle checkbox"));
            lines.push(("  Enter ", "Save"));
            lines.push(("  Esc   ", "Cancel"));
        }
        4 => {
            lines.push(("── Usage ──", ""));
            lines.push(("  ←/→   ", "Switch sub-tab"));
            lines.push(("  +/-   ", "Adjust date range"));
            lines.push(("  r     ", "Refresh data"));
            lines.push(("  x     ", "Export CSV"));
            lines.push(("  j/k   ", "Scroll"));
        }
        5 => {
            lines.push(("── Settings ──", ""));
            lines.push(("  j/k   ", "Navigate fields"));
            lines.push(("  Enter ", "Edit / Activate"));
            lines.push(("  s     ", "Save settings"));
            lines.push(("  ←/→   ", "Change log level"));
        }
        _ => {}
    }

    lines
}

pub fn render(frame: &mut Frame, area: Rect, active_tab: usize) {
    let lines = help_lines_for_page(active_tab);

    let max_line_width = lines
        .iter()
        .map(|(key, desc)| {
            if desc.is_empty() {
                key.len()
            } else {
                key.len() + desc.len()
            }
        })
        .max()
        .unwrap_or(30);
    let width = (max_line_width as u16 + 4).min(area.width).max(36);
    let height = (lines.len() as u16 + 2).min(area.height);
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup_area = Rect::new(x, y, width, height);

    let page_name = TAB_NAMES.get(active_tab).unwrap_or(&"Help");

    let block = Block::default()
        .title(format!(" Key Bindings — {} ", page_name))
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Black));

    let rendered_lines: Vec<Line> = lines
        .iter()
        .map(|(key, desc)| {
            if desc.is_empty() && key.starts_with("──") {
                Line::from(Span::styled(
                    format!(" {}", key),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ))
            } else if desc.is_empty() {
                Line::from("")
            } else {
                Line::from(vec![
                    Span::styled(
                        format!(" {}", key),
                        Style::default().fg(Color::Yellow),
                    ),
                    Span::raw(*desc),
                ])
            }
        })
        .collect();

    let paragraph = Paragraph::new(rendered_lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(Clear, popup_area);
    frame.render_widget(paragraph, popup_area);
}
