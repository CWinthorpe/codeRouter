use std::sync::{Mutex, OnceLock};

use crate::app::{App, ToastMessage, VERSION};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use coderouter_proxy::config::store;
use coderouter_proxy::metrics::db;

const LOG_LEVELS: &[&str] = &["Trace", "Debug", "Info", "Warn", "Error"];
const TOTAL_FIELDS: usize = 8;

#[derive(Clone, Copy, PartialEq)]
enum SettingsMode {
    View,
    Editing,
    ConfirmClear,
    ConfirmReset,
    ConfirmResetSecond,
}

struct SettingsState {
    proxy_port: String,
    proxy_host: String,
    refresh_interval: String,
    log_verbosity: usize,
    focused_field: usize,
    mode: SettingsMode,
    config_dir_display: String,
    metrics_db_display: String,
}

static STATE: OnceLock<Mutex<Option<SettingsState>>> = OnceLock::new();

fn load_state() -> SettingsState {
    let config = store::load_app_config().unwrap_or_default();
    let log_idx = LOG_LEVELS
        .iter()
        .position(|&l| l.eq_ignore_ascii_case(&config.log_verbosity))
        .unwrap_or(2);

    let config_dir_display = store::app_config_path()
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let metrics_db_display = db::db_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    SettingsState {
        proxy_port: config.proxy_port.to_string(),
        proxy_host: config.proxy_host,
        refresh_interval: config.refresh_interval_hours.to_string(),
        log_verbosity: log_idx,
        focused_field: 0,
        mode: SettingsMode::View,
        config_dir_display,
        metrics_db_display,
    }
}

fn ensure_loaded() {
    STATE.get_or_init(|| Mutex::new(Some(load_state())));
}

pub fn render(_app: &App, frame: &mut Frame, area: Rect) {
    ensure_loaded();
    let state_ref = match STATE.get() {
        Some(s) => s,
        None => return,
    };
    let mut guard = match state_ref.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let state = match guard.as_mut() {
        Some(s) => s,
        None => return,
    };

    render_main(frame, area, state);

    match state.mode {
        SettingsMode::ConfirmClear => {
            render_confirm(frame, area, "Clear all metrics data?");
        }
        SettingsMode::ConfirmReset => {
            render_confirm(frame, area, "Reset ALL configuration to defaults?");
        }
        SettingsMode::ConfirmResetSecond => {
            render_confirm(frame, area, "Are you REALLY sure? Press y again to confirm.");
        }
        _ => {}
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) {
    let state_ref = match STATE.get() {
        Some(s) => s,
        None => return,
    };
    let mut guard = match state_ref.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let state = match guard.as_mut() {
        Some(s) => s,
        None => return,
    };

    match state.mode {
        SettingsMode::View => handle_view(app, key, state),
        SettingsMode::Editing => handle_editing(key, state),
        SettingsMode::ConfirmClear => handle_confirm_clear(app, key, state),
        SettingsMode::ConfirmReset | SettingsMode::ConfirmResetSecond => {
            handle_confirm_reset(app, key, state)
        }
    }
}

fn render_main(frame: &mut Frame, area: Rect, state: &SettingsState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            " Settings ",
            Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan),
        ));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let is_editing = state.mode == SettingsMode::Editing;
    let focused = state.focused_field;

    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(Span::styled(
        " ── Proxy Settings ──",
        Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan),
    )));
    lines.push(Line::from(""));

    lines.push(make_field_line(
        "Port",
        &field_value(&state.proxy_port, focused == 0, is_editing),
        focused == 0,
        is_editing,
    ));
    lines.push(make_field_line(
        "Host",
        &field_value(&state.proxy_host, focused == 1, is_editing),
        focused == 1,
        is_editing,
    ));
    lines.push(make_field_line(
        "Refresh",
        &field_value(
            &format!("{} hours", state.refresh_interval),
            focused == 2,
            is_editing,
        ),
        focused == 2,
        is_editing,
    ));

    let verb = if focused == 3 && is_editing {
        format!("< {} >", LOG_LEVELS[state.log_verbosity])
    } else {
        LOG_LEVELS[state.log_verbosity].to_string()
    };
    lines.push(make_field_line("Verbosity", &verb, focused == 3, is_editing));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " ── Actions ──",
        Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan),
    )));
    lines.push(Line::from(""));

    lines.push(make_button_line("[Save Settings]", focused == 4));
    lines.push(make_button_line("[Clear Metrics]", focused == 5));
    lines.push(make_button_line("[Reset All Config]", focused == 6));
    lines.push(make_button_line("[Restart Proxy]", focused == 7));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " ── About ──",
        Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan),
    )));
    lines.push(Line::from(""));

    lines.push(Line::from(vec![
        Span::styled("   Version:  ", Style::default().fg(Color::DarkGray)),
        Span::styled(VERSION, Style::default().fg(Color::White)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("   Config:   ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            state.config_dir_display.as_str(),
            Style::default().fg(Color::White),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("   Metrics:  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            state.metrics_db_display.as_str(),
            Style::default().fg(Color::White),
        ),
    ]));

    lines.push(Line::from(""));

    let hint = if is_editing {
        match focused {
            3 => " ←/→:Change  Enter:Confirm  Esc:Cancel ",
            _ => " Type to edit  Enter:Confirm  Esc:Cancel ",
        }
    } else {
        " j/k:Nav  Enter:Edit/Activate  s:Save  ?:Help "
    };
    lines.push(Line::from(Span::styled(
        hint,
        Style::default().fg(Color::DarkGray),
    )));

    frame.render_widget(Paragraph::new(lines), inner);
}

fn field_value(val: &str, focused: bool, editing: bool) -> String {
    if focused && editing {
        format!("{}_", val)
    } else {
        val.to_string()
    }
}

fn make_field_line(label: &str, value: &str, focused: bool, editing: bool) -> Line<'static> {
    let label_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let value_style = if focused && editing {
        Style::default().fg(Color::White).bg(Color::DarkGray)
    } else if focused {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    Line::from(vec![
        Span::styled(format!("   {:12}", format!("{}:", label)), label_style),
        Span::styled(value.to_string(), value_style),
    ])
}

fn make_button_line(label: &str, focused: bool) -> Line<'static> {
    let style = if focused {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    Line::from(Span::styled(format!("   {}", label), style))
}

fn render_confirm(frame: &mut Frame, area: Rect, msg: &str) {
    let width = (msg.len() as u16 + 6).min(area.width).max(40);
    let height = 5.min(area.height);
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup_area = Rect::new(x, y, width, height);

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" Confirm ")
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Span::styled(msg, Style::default().fg(Color::White)))
            .alignment(Alignment::Center),
        chunks[0],
    );
    frame.render_widget(Paragraph::new(""), chunks[1]);
    frame.render_widget(
        Paragraph::new(Span::styled(
            " y:Confirm  Esc:Cancel ",
            Style::default().fg(Color::DarkGray),
        ))
        .alignment(Alignment::Center),
        chunks[2],
    );
}

fn handle_view(app: &mut App, key: KeyEvent, state: &mut SettingsState) {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            state.focused_field = (state.focused_field + 1) % TOTAL_FIELDS;
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.focused_field = if state.focused_field == 0 {
                TOTAL_FIELDS - 1
            } else {
                state.focused_field - 1
            };
        }
        KeyCode::Char('s') => {
            do_save(app, state);
        }
        KeyCode::Enter => match state.focused_field {
            0..=3 => {
                state.mode = SettingsMode::Editing;
            }
            4 => do_save(app, state),
            5 => state.mode = SettingsMode::ConfirmClear,
            6 => state.mode = SettingsMode::ConfirmReset,
            7 => do_restart_proxy(app),
            _ => {}
        },
        _ => {}
    }
}

fn handle_editing(key: KeyEvent, state: &mut SettingsState) {
    match state.focused_field {
        0 | 2 => match key.code {
            KeyCode::Char(c) if c.is_ascii_digit() => {
                let buf = if state.focused_field == 0 {
                    &mut state.proxy_port
                } else {
                    &mut state.refresh_interval
                };
                if buf.len() < 6 {
                    buf.push(c);
                }
            }
            KeyCode::Backspace => {
                let buf = if state.focused_field == 0 {
                    &mut state.proxy_port
                } else {
                    &mut state.refresh_interval
                };
                buf.pop();
            }
            KeyCode::Enter => {
                state.mode = SettingsMode::View;
            }
            KeyCode::Esc => {
                let config = store::load_app_config().unwrap_or_default();
                match state.focused_field {
                    0 => state.proxy_port = config.proxy_port.to_string(),
                    2 => state.refresh_interval = config.refresh_interval_hours.to_string(),
                    _ => {}
                }
                state.mode = SettingsMode::View;
            }
            _ => {}
        },
        1 => match key.code {
            KeyCode::Char(c) => {
                if state.proxy_host.len() < 64 {
                    state.proxy_host.push(c);
                }
            }
            KeyCode::Backspace => {
                state.proxy_host.pop();
            }
            KeyCode::Enter => {
                state.mode = SettingsMode::View;
            }
            KeyCode::Esc => {
                let config = store::load_app_config().unwrap_or_default();
                state.proxy_host = config.proxy_host;
                state.mode = SettingsMode::View;
            }
            _ => {}
        },
        3 => match key.code {
            KeyCode::Char('j') | KeyCode::Down | KeyCode::Right => {
                state.log_verbosity = (state.log_verbosity + 1) % LOG_LEVELS.len();
            }
            KeyCode::Char('k') | KeyCode::Up | KeyCode::Left => {
                state.log_verbosity = if state.log_verbosity == 0 {
                    LOG_LEVELS.len() - 1
                } else {
                    state.log_verbosity - 1
                };
            }
            KeyCode::Enter => {
                state.mode = SettingsMode::View;
            }
            KeyCode::Esc => {
                let config = store::load_app_config().unwrap_or_default();
                state.log_verbosity = LOG_LEVELS
                    .iter()
                    .position(|&l| l.eq_ignore_ascii_case(&config.log_verbosity))
                    .unwrap_or(2);
                state.mode = SettingsMode::View;
            }
            _ => {}
        },
        _ => {
            state.mode = SettingsMode::View;
        }
    }
}

fn handle_confirm_clear(app: &mut App, key: KeyEvent, state: &mut SettingsState) {
    match key.code {
        KeyCode::Char('y') => {
            match db::clear_metrics() {
                Ok(()) => app.toast = Some(ToastMessage::new("Metrics cleared")),
                Err(e) => app.toast = Some(ToastMessage::new(format!("Error: {}", e))),
            }
            state.mode = SettingsMode::View;
        }
        KeyCode::Esc => {
            state.mode = SettingsMode::View;
        }
        _ => {}
    }
}

fn handle_confirm_reset(app: &mut App, key: KeyEvent, state: &mut SettingsState) {
    match key.code {
        KeyCode::Char('y') => {
            if state.mode == SettingsMode::ConfirmReset {
                state.mode = SettingsMode::ConfirmResetSecond;
            } else {
                match store::reset_all_config() {
                    Ok(()) => {
                        app.toast = Some(ToastMessage::new("Config reset to defaults"));
                        let new = load_state();
                        state.proxy_port = new.proxy_port;
                        state.proxy_host = new.proxy_host;
                        state.refresh_interval = new.refresh_interval;
                        state.log_verbosity = new.log_verbosity;
                        state.mode = SettingsMode::View;
                    }
                    Err(e) => {
                        app.toast = Some(ToastMessage::new(format!("Error: {}", e)));
                        state.mode = SettingsMode::View;
                    }
                }
            }
        }
        KeyCode::Esc => {
            state.mode = SettingsMode::View;
        }
        _ => {}
    }
}

fn do_save(app: &mut App, state: &mut SettingsState) {
    let port: u16 = match state.proxy_port.parse() {
        Ok(v) => v,
        Err(_) => {
            app.toast = Some(ToastMessage::new("Invalid port number"));
            return;
        }
    };
    let interval: u32 = match state.refresh_interval.parse() {
        Ok(v) => v,
        Err(_) => {
            app.toast = Some(ToastMessage::new("Invalid refresh interval"));
            return;
        }
    };
    if state.proxy_host.is_empty() {
        app.toast = Some(ToastMessage::new("Host cannot be empty"));
        return;
    }
    let log_verbosity = LOG_LEVELS[state.log_verbosity].to_string();

    let mut config = store::load_app_config().unwrap_or_default();
    config.proxy_port = port;
    config.proxy_host = state.proxy_host.clone();
    config.refresh_interval_hours = interval;
    config.log_verbosity = log_verbosity;

    match store::save_app_config(&config) {
        Ok(()) => app.toast = Some(ToastMessage::new("Settings saved")),
        Err(e) => app.toast = Some(ToastMessage::new(format!("Error: {}", e))),
    }
}

fn do_restart_proxy(app: &mut App) {
    if let Some(ref mut child) = app.sidecar {
        kill_sidecar(child);
    }

    match spawn_sidecar() {
        Ok(new_child) => {
            app.sidecar = Some(new_child);
            app.proxy_running = true;
            app.toast = Some(ToastMessage::new("Proxy restarted"));
        }
        Err(e) => {
            app.sidecar = None;
            app.proxy_running = false;
            app.toast = Some(ToastMessage::new(format!("Failed to restart proxy: {}", e)));
        }
    }
}

fn spawn_sidecar() -> std::result::Result<std::process::Child, String> {
    let exe_name = if cfg!(debug_assertions) {
        let manifest_dir =
            std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
        let dev_path = std::path::Path::new(&manifest_dir)
            .join("../sidecar/target/debug/coderouter-proxy");
        if dev_path.exists() {
            return std::process::Command::new(&dev_path)
                .spawn()
                .map_err(|e| e.to_string());
        }
        "coderouter-proxy".to_string()
    } else {
        "coderouter-proxy".to_string()
    };

    std::process::Command::new(&exe_name)
        .spawn()
        .map_err(|e| e.to_string())
}

fn kill_sidecar(child: &mut std::process::Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
            Err(_) => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}
