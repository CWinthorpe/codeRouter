use crate::pages;
use crate::widgets;
use crossterm::event::KeyEvent;
use ratatui::prelude::*;
use std::process::Child;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub const TAB_NAMES: &[&str] = &["Dashboard", "Providers", "Groups", "OpenCode", "Usage", "Settings"];

pub struct ToastMessage {
    pub text: String,
    pub created_at: std::time::Instant,
}

impl ToastMessage {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            created_at: std::time::Instant::now(),
        }
    }

    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed().as_secs() > 3
    }
}

pub struct App {
    pub active_tab: usize,
    pub proxy_running: bool,
    pub sidecar: Option<Child>,
    pub toast: Option<ToastMessage>,
    pub should_quit: bool,
    pub show_help: bool,
}

impl App {
    pub fn new(sidecar: Option<Child>) -> Self {
        Self {
            active_tab: 0,
            proxy_running: sidecar.is_some(),
            sidecar,
            toast: None,
            should_quit: false,
            show_help: false,
        }
    }

    pub fn render(&mut self, frame: &mut Frame) {
        if let Some(ref toast) = self.toast {
            if toast.is_expired() {
                self.toast = None;
            }
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(frame.area());

        widgets::tab_bar::render(frame, chunks[0], self.active_tab);

        pages::render_page(self, frame, chunks[1]);

        widgets::status_bar::render(frame, chunks[2], self.proxy_running);

        if let Some(ref toast) = self.toast {
            widgets::toast::render(frame, frame.area(), &toast.text);
        }

        if self.show_help {
            widgets::help_overlay::render(frame, frame.area(), self.active_tab);
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if self.show_help {
            match key.code {
                crossterm::event::KeyCode::Char('?') | crossterm::event::KeyCode::Esc => {
                    self.show_help = false;
                }
                _ => {}
            }
            return;
        }

        match key.code {
            crossterm::event::KeyCode::Char('1') => self.active_tab = 0,
            crossterm::event::KeyCode::Char('2') => self.active_tab = 1,
            crossterm::event::KeyCode::Char('3') => self.active_tab = 2,
            crossterm::event::KeyCode::Char('4') => self.active_tab = 3,
            crossterm::event::KeyCode::Char('5') => self.active_tab = 4,
            crossterm::event::KeyCode::Char('6') => self.active_tab = 5,
            crossterm::event::KeyCode::Tab => {
                self.active_tab = (self.active_tab + 1) % TAB_NAMES.len();
            }
            crossterm::event::KeyCode::BackTab => {
                if self.active_tab == 0 {
                    self.active_tab = TAB_NAMES.len() - 1;
                } else {
                    self.active_tab -= 1;
                }
            }
            crossterm::event::KeyCode::Char('q') => {
                self.should_quit = true;
            }
            crossterm::event::KeyCode::Char('?') => {
                self.show_help = true;
            }
            _ => {
                pages::handle_key(self, key);
            }
        }
    }
}
