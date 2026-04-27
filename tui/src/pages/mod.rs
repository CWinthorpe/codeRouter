pub mod dashboard;
pub mod groups;
pub mod opencode;
pub mod providers;
pub mod settings;
pub mod usage;

use crate::app::App;
use crossterm::event::KeyEvent;
use ratatui::prelude::*;

pub fn render_page(app: &App, frame: &mut Frame, area: Rect) {
    match app.active_tab {
        0 => dashboard::render(app, frame, area),
        1 => providers::render(app, frame, area),
        2 => groups::render(app, frame, area),
        3 => opencode::render(app, frame, area),
        4 => usage::render(app, frame, area),
        5 => settings::render(app, frame, area),
        _ => {}
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) {
    match app.active_tab {
        0 => dashboard::handle_key(app, key),
        1 => providers::handle_key(app, key),
        2 => groups::handle_key(app, key),
        3 => opencode::handle_key(app, key),
        4 => usage::handle_key(app, key),
        5 => settings::handle_key(app, key),
        _ => {}
    }
}
