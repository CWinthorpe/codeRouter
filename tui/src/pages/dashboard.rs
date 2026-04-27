use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::app::{App, VERSION};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use coderouter_proxy::config::store;
use coderouter_proxy::metrics::{db, queries};
use coderouter_proxy::proxy::router::{self, EntryStatusResponse};

#[derive(Clone)]
struct ProxyInfo {
    running: bool,
    uptime_seconds: Option<u64>,
    host: String,
    port: u16,
}

#[derive(Clone)]
struct ProviderHealthInfo {
    name: String,
    enabled: bool,
    status_label: String,
    status_color: Color,
    weekly_cost: f64,
    monthly_cost: f64,
    error_count: i64,
    total_cost_today: f64,
}

#[derive(Clone)]
struct DashboardData {
    proxy: ProxyInfo,
    providers: Vec<ProviderHealthInfo>,
    recent_requests: Vec<queries::RequestRow>,
    recent_errors: Vec<queries::RequestRow>,
}

static DATA: OnceLock<Mutex<Option<DashboardData>>> = OnceLock::new();
static SCROLL: AtomicUsize = AtomicUsize::new(0);
static REFRESH: AtomicBool = AtomicBool::new(false);

fn ensure_polling() {
    DATA.get_or_init(|| {
        std::thread::Builder::new()
            .name("dashboard-poll".into())
            .spawn(poll_loop)
            .expect("failed to spawn dashboard poll thread");
        Mutex::new(None)
    });
}

fn poll_loop() {
    let mut conn = loop {
        match db::init_db() {
            Ok(c) => break c,
            Err(_) => std::thread::sleep(std::time::Duration::from_secs(5)),
        }
    };

    loop {
        let data = fetch_dashboard_data(&conn);
        if let Some(state) = DATA.get() {
            if let Ok(mut guard) = state.lock() {
                *guard = Some(data);
            }
        }

        let start = std::time::Instant::now();
        loop {
            if REFRESH.swap(false, Ordering::Relaxed) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
            if start.elapsed() >= std::time::Duration::from_secs(5) {
                break;
            }
        }

        if let Ok(new_conn) = db::init_db() {
            conn = new_conn;
        }
    }
}

fn fetch_dashboard_data(conn: &rusqlite::Connection) -> DashboardData {
    let app_config = store::load_app_config().unwrap_or_default();
    let providers = store::load_providers().unwrap_or_default();

    let (proxy_running, uptime) = check_proxy_health(&app_config.proxy_host, app_config.proxy_port);
    let router_entries = fetch_router_entries(&app_config.proxy_host, app_config.proxy_port);

    let today = chrono::Utc::now().date_naive();
    let mut provider_infos: Vec<ProviderHealthInfo> = providers
        .iter()
        .map(|provider| {
            let reset_hour = provider.quota_reset_utc_hour;
            let summary = queries::get_daily_summary(conn, &provider.id, today, reset_hour).ok();
            let weekly_cost = queries::get_cost_summary(conn, &provider.id, 7, reset_hour).unwrap_or(0.0);
            let monthly_cost = queries::get_cost_summary(conn, &provider.id, 30, reset_hour).unwrap_or(0.0);
            let (status_label, status_color) = determine_provider_status(provider, &router_entries);

            ProviderHealthInfo {
                name: provider.name.clone(),
                enabled: provider.enabled,
                status_label,
                status_color,
                weekly_cost,
                monthly_cost,
                error_count: summary.as_ref().map(|s| s.error_count).unwrap_or(0),
                total_cost_today: summary.as_ref().map(|s| s.total_cost).unwrap_or(0.0),
            }
        })
        .collect();

    provider_infos.sort_by(|a, b| {
        let key = |p: &ProviderHealthInfo| -> u8 {
            if !p.enabled {
                return 2;
            }
            if p.status_label == "Active" {
                return 1;
            }
            0
        };
        key(a).cmp(&key(b))
    });

    let recent_requests = queries::get_recent_requests(conn, 15).unwrap_or_default();
    let recent_errors: Vec<_> = queries::get_recent_requests(conn, 100)
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.status != "success")
        .take(10)
        .collect();

    DashboardData {
        proxy: ProxyInfo {
            running: proxy_running,
            uptime_seconds: uptime,
            host: app_config.proxy_host,
            port: app_config.proxy_port,
        },
        providers: provider_infos,
        recent_requests,
        recent_errors,
    }
}

fn check_proxy_health(host: &str, port: u16) -> (bool, Option<u64>) {
    let url = format!("http://{}:{}/health", host, port);
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap_or_default();

    match client.get(&url).send() {
        Ok(resp) if resp.status().is_success() => {
            let body: serde_json::Value = resp.json().unwrap_or_default();
            let uptime = body.get("uptime_seconds").and_then(|v| v.as_u64());
            (true, uptime)
        }
        _ => (false, None),
    }
}

fn fetch_router_entries(host: &str, port: u16) -> Vec<EntryStatusResponse> {
    let url = format!("http://{}:{}/internal/router/status", host, port);
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap_or_default();

    let resp = match client.get(&url).send() {
        Ok(r) => r,
        Err(_) => return vec![],
    };

    if !resp.status().is_success() {
        return vec![];
    }

    let body: serde_json::Value = match resp.json() {
        Ok(b) => b,
        Err(_) => return vec![],
    };

    let data = match body.get("data") {
        Some(d) => d,
        None => return vec![],
    };

    match serde_json::from_value::<router::RouterStatusResponse>(data.clone()) {
        Ok(status) => status.entries,
        Err(_) => vec![],
    }
}

fn determine_provider_status(
    provider: &coderouter_proxy::config::models::Provider,
    entries: &[EntryStatusResponse],
) -> (String, Color) {
    if !provider.enabled {
        return ("Disabled".to_string(), Color::DarkGray);
    }

    let provider_entries: Vec<_> = entries.iter().filter(|e| e.provider_id == provider.id).collect();
    if provider_entries.is_empty() {
        return ("Active".to_string(), Color::Green);
    }

    let has_cooldown = provider_entries.iter().any(|e| e.status == "cooldown");
    let has_quota_exhausted = provider_entries.iter().any(|e| e.status == "quota_exhausted");
    let has_disabled = provider_entries.iter().any(|e| e.status == "manually_disabled");
    let all_non_active = provider_entries.iter().all(|e| e.status != "active");

    if has_quota_exhausted {
        return ("Quota Exhausted".to_string(), Color::Red);
    }
    if all_non_active {
        return ("All Exhausted".to_string(), Color::Red);
    }
    if has_cooldown {
        return ("Degraded".to_string(), Color::Yellow);
    }
    if has_disabled {
        return ("Partial".to_string(), Color::Yellow);
    }
    ("Active".to_string(), Color::Green)
}

pub fn render(app: &App, frame: &mut Frame, area: Rect) {
    ensure_polling();

    let data = DATA.get().and_then(|m| m.lock().ok()).and_then(|g| g.clone());

    match data {
        Some(d) => render_dashboard(frame, area, app, &d),
        None => render_loading(frame, area),
    }
}

fn render_loading(frame: &mut Frame, area: Rect) {
    let text = vec![
        Line::from(Span::styled(
            format!("CodeRouter v{VERSION} Dashboard"),
            Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Loading data...",
            Style::default().fg(Color::Yellow),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "r: Refresh  j/k: Scroll  ?: Help  q: Quit",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    frame.render_widget(Paragraph::new(text), area);
}

fn render_dashboard(frame: &mut Frame, area: Rect, app: &App, data: &DashboardData) {
    let card_height: u16 = 5;
    let n = data.providers.len().max(1);
    let card_rows = ((n + 1) / 2) as u16;
    let provider_section_height = card_rows * card_height + 2;

    let error_section_height = if data.recent_errors.is_empty() {
        0u16
    } else {
        data.recent_errors.len().min(10) as u16 + 3
    };

    let mut constraints: Vec<Constraint> = vec![
        Constraint::Length(5),
        Constraint::Length(provider_section_height),
    ];

    if error_section_height > 0 {
        constraints.push(Constraint::Length(error_section_height));
    }
    constraints.push(Constraint::Min(8));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    let mut idx = 0;
    render_proxy_status(frame, chunks[idx], app, &data.proxy);
    idx += 1;
    render_provider_cards(frame, chunks[idx], &data.providers);
    idx += 1;
    if error_section_height > 0 && idx < chunks.len() {
        render_recent_errors(frame, chunks[idx], &data.recent_errors);
        idx += 1;
    }
    if idx < chunks.len() {
        render_recent_requests(frame, chunks[idx], &data.recent_requests);
    }
}

fn render_proxy_status(frame: &mut Frame, area: Rect, _app: &App, proxy: &ProxyInfo) {
    let (status_text, status_color) = if proxy.running {
        ("● Running", Color::Green)
    } else {
        ("● Stopped", Color::Red)
    };

    let mut info_spans: Vec<Span> = vec![Span::styled(
        format!("  {}:{}", proxy.host, proxy.port),
        Style::default().fg(Color::DarkGray),
    )];

    if let Some(uptime) = proxy.uptime_seconds {
        info_spans.push(Span::raw("  "));
        info_spans.push(Span::styled(
            format!("Uptime: {}", format_uptime(uptime)),
            Style::default().fg(Color::DarkGray),
        ));
    }

    let lines = vec![
        Line::from(vec![
            Span::styled(
                " Proxy Status ",
                Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan),
            ),
            Span::raw("  "),
            Span::styled(
                status_text,
                Style::default().fg(status_color).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(info_spans),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn render_provider_cards(frame: &mut Frame, area: Rect, providers: &[ProviderHealthInfo]) {
    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            " Provider Health ",
            Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan),
        ));

    let inner = outer_block.inner(area);
    frame.render_widget(outer_block, area);

    if providers.is_empty() {
        let msg = Paragraph::new("No providers configured")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(msg, inner);
        return;
    }

    let cols: u16 = if providers.len() == 1 { 1 } else { 2 };
    let col_width = inner.width / cols;

    for (i, provider) in providers.iter().enumerate() {
        let col = i as u16 % cols;
        let row = i as u16 / cols;
        let x = inner.x + col * col_width;
        let y = inner.y + row * 5;
        let card_area = Rect::new(x, y, col_width.saturating_sub(1), 5);

        if card_area.width < 10 || card_area.bottom() > inner.bottom() {
            continue;
        }

        render_single_provider_card(frame, card_area, provider);
    }
}

fn render_single_provider_card(frame: &mut Frame, area: Rect, provider: &ProviderHealthInfo) {
    let status_badge = Span::styled(
        format!(" {} ", provider.status_label),
        Style::default()
            .fg(Color::Black)
            .bg(provider.status_color)
            .add_modifier(Modifier::BOLD),
    );

    let name_span = Span::styled(
        format!(" {}", provider.name),
        Style::default().add_modifier(Modifier::BOLD),
    );

    let line1 = Line::from(vec![name_span, Span::raw(" "), status_badge]);

    let cost_text = format!(
        " 7d: {}  30d: {}",
        format_cost(provider.weekly_cost),
        format_cost(provider.monthly_cost),
    );
    let line2 = Line::from(Span::styled(
        cost_text,
        Style::default().fg(Color::DarkGray),
    ));

    let mut line3_parts = vec![];
    if provider.error_count > 0 {
        line3_parts.push(Span::styled(
            format!(" Errors: {}", provider.error_count),
            Style::default().fg(Color::Red),
        ));
    }
    if provider.total_cost_today > 0.0 {
        if !line3_parts.is_empty() {
            line3_parts.push(Span::raw("  "));
        }
        line3_parts.push(Span::styled(
            format!("Today: {}", format_cost(provider.total_cost_today)),
            Style::default().fg(Color::DarkGray),
        ));
    }
    let line3 = Line::from(line3_parts);

    let card_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let content = Paragraph::new(vec![line1, line2, line3]).block(card_block);
    frame.render_widget(content, area);
}

fn render_recent_errors(frame: &mut Frame, area: Rect, errors: &[queries::RequestRow]) {
    let header = Row::new(vec![
        Cell::from("Time").style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
        Cell::from("Provider").style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
        Cell::from("Model").style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
        Cell::from("Error").style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
    ])
    .bottom_margin(1);

    let rows: Vec<Row> = errors
        .iter()
        .map(|e| {
            Row::new(vec![
                Cell::from(format_relative_time(e.ts)).style(Style::default().fg(Color::DarkGray)),
                Cell::from(e.provider_id.clone()),
                Cell::from(e.model_id.clone()).style(Style::default().fg(Color::DarkGray)),
                Cell::from(e.error_type.as_deref().unwrap_or("unknown"))
                    .style(Style::default().fg(Color::Red)),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Length(15),
            Constraint::Min(15),
            Constraint::Length(15),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Span::styled(
                " Recent Errors ",
                Style::default().add_modifier(Modifier::BOLD).fg(Color::Red),
            )),
    );
    frame.render_widget(table, area);
}

fn render_recent_requests(frame: &mut Frame, area: Rect, requests: &[queries::RequestRow]) {
    let scroll = SCROLL.load(Ordering::Relaxed);
    let header_height = 2u16;
    let visible_height = area.height.saturating_sub(2 + header_height) as usize;
    let max_scroll = requests.len().saturating_sub(visible_height);
    let clamped_scroll = scroll.min(max_scroll);
    SCROLL.store(clamped_scroll, Ordering::Relaxed);

    let visible: Vec<_> = requests
        .iter()
        .skip(clamped_scroll)
        .take(visible_height)
        .collect();

    let header = Row::new(vec![
        Cell::from("Time").style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
        Cell::from("Group").style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
        Cell::from("Provider").style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
        Cell::from("Latency").style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
        Cell::from("Status").style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
    ])
    .bottom_margin(1);

    let rows: Vec<Row> = visible
        .iter()
        .map(|r| {
            let (status_text, status_color) = if r.status == "success" {
                ("✓".to_string(), Color::Green)
            } else {
                (
                    r.error_type.as_deref().unwrap_or("✗").to_string(),
                    Color::Red,
                )
            };

            Row::new(vec![
                Cell::from(format_relative_time(r.ts)).style(Style::default().fg(Color::DarkGray)),
                Cell::from(r.group_alias.clone()),
                Cell::from(r.provider_id.clone()).style(Style::default().fg(Color::DarkGray)),
                Cell::from(format_latency(r.latency_ms)).style(Style::default().fg(Color::DarkGray)),
                Cell::from(status_text).style(Style::default().fg(status_color)),
            ])
        })
        .collect();

    let shown_end = (clamped_scroll + visible.len()).min(requests.len());
    let title = if requests.is_empty() {
        " Recent Requests ".to_string()
    } else {
        format!(
            " Recent Requests ({}/{}) ",
            shown_end, requests.len()
        )
    };

    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Length(15),
            Constraint::Length(15),
            Constraint::Length(10),
            Constraint::Length(10),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Span::styled(
                title,
                Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan),
            )),
    );
    frame.render_widget(table, area);
}

pub fn handle_key(_app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('r') => {
            REFRESH.store(true, Ordering::Relaxed);
        }
        KeyCode::Char('j') | KeyCode::Down => {
            let current = SCROLL.load(Ordering::Relaxed);
            SCROLL.store(current.saturating_add(1), Ordering::Relaxed);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let current = SCROLL.load(Ordering::Relaxed);
            SCROLL.store(current.saturating_sub(1), Ordering::Relaxed);
        }
        _ => {}
    }
}

fn format_uptime(seconds: u64) -> String {
    if seconds < 60 {
        format!("{}s", seconds)
    } else if seconds < 3600 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else if seconds < 86400 {
        format!("{}h {}m", seconds / 3600, (seconds % 3600) / 60)
    } else {
        format!("{}d {}h", seconds / 86400, (seconds % 86400) / 3600)
    }
}

fn format_relative_time(ts: i64) -> String {
    let now = chrono::Utc::now().timestamp();
    let diff = now.saturating_sub(ts);
    if diff <= 0 {
        return "now".to_string();
    }
    if diff < 60 {
        format!("{}s", diff)
    } else if diff < 3600 {
        format!("{}m", diff / 60)
    } else if diff < 86400 {
        format!("{}h", diff / 3600)
    } else {
        format!("{}d", diff / 86400)
    }
}

fn format_cost(cost: f64) -> String {
    if cost == 0.0 {
        "$0".to_string()
    } else if cost < 0.01 {
        format!("${:.4}", cost)
    } else {
        format!("${:.2}", cost)
    }
}

fn format_latency(ms: i64) -> String {
    if ms <= 0 {
        "—".to_string()
    } else if ms < 1000 {
        format!("{}ms", ms)
    } else {
        format!("{:.1}s", ms as f64 / 1000.0)
    }
}
