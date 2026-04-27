use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::app::{App, ToastMessage};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use coderouter_proxy::metrics::db;

const SUB_TABS: &[&str] = &["Daily Summary", "By Provider", "By Group", "By Model"];

#[derive(Clone)]
struct UsageRow {
    label: String,
    cost: f64,
    input_tokens: i64,
    output_tokens: i64,
    requests: i64,
}

#[derive(Clone)]
struct UsageData {
    sub_tab: usize,
    days_back: u32,
    rows: Vec<UsageRow>,
}

static DATA: OnceLock<Mutex<Option<UsageData>>> = OnceLock::new();
static SUB_TAB: AtomicUsize = AtomicUsize::new(0);
static DAYS_BACK: AtomicUsize = AtomicUsize::new(7);
static SCROLL: AtomicUsize = AtomicUsize::new(0);
static REFRESH: AtomicBool = AtomicBool::new(false);

fn ensure_polling() {
    DATA.get_or_init(|| {
        std::thread::Builder::new()
            .name("usage-poll".into())
            .spawn(poll_loop)
            .expect("failed to spawn usage poll thread");
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
        let sub_tab = SUB_TAB.load(Ordering::Relaxed);
        let days_back = DAYS_BACK.load(Ordering::Relaxed) as u32;

        let rows = fetch_data(&conn, sub_tab, days_back);
        let data = UsageData {
            sub_tab,
            days_back,
            rows,
        };

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
            let cur_sub = SUB_TAB.load(Ordering::Relaxed);
            let cur_days = DAYS_BACK.load(Ordering::Relaxed) as u32;
            if cur_sub != sub_tab || cur_days != days_back {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
            if start.elapsed() >= std::time::Duration::from_secs(10) {
                break;
            }
        }

        if let Ok(new_conn) = db::init_db() {
            conn = new_conn;
        }
    }
}

fn fetch_data(conn: &rusqlite::Connection, sub_tab: usize, days_back: u32) -> Vec<UsageRow> {
    let now = chrono::Utc::now();
    let start_ts = (now - chrono::Duration::days(days_back as i64))
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp();

    match sub_tab {
        0 => fetch_daily_summary(conn, start_ts),
        1 => fetch_by_provider(conn, start_ts),
        2 => fetch_by_group(conn, start_ts),
        3 => fetch_by_model(conn, start_ts),
        _ => vec![],
    }
}

fn fetch_daily_summary(conn: &rusqlite::Connection, start_ts: i64) -> Vec<UsageRow> {
    let mut stmt = match conn.prepare(
        "SELECT DATE(ts, 'unixepoch'),
                COALESCE(SUM(cost_usd), 0.0),
                COALESCE(SUM(prompt_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                COUNT(*)
         FROM requests WHERE ts >= ?1
         GROUP BY DATE(ts, 'unixepoch')
         ORDER BY COALESCE(SUM(cost_usd), 0.0) DESC",
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    stmt.query_map([&start_ts.to_string()], |row| {
        Ok(UsageRow {
            label: row.get(0)?,
            cost: row.get(1)?,
            input_tokens: row.get(2)?,
            output_tokens: row.get(3)?,
            requests: row.get(4)?,
        })
    })
    .ok()
    .map(|iter| iter.filter_map(|r| r.ok()).collect())
    .unwrap_or_default()
}

fn fetch_by_provider(conn: &rusqlite::Connection, start_ts: i64) -> Vec<UsageRow> {
    let mut stmt = match conn.prepare(
        "SELECT provider_id,
                COALESCE(SUM(cost_usd), 0.0),
                COALESCE(SUM(prompt_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                COUNT(*)
         FROM requests WHERE ts >= ?1
         GROUP BY provider_id
         ORDER BY COALESCE(SUM(cost_usd), 0.0) DESC",
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    stmt.query_map([&start_ts.to_string()], |row| {
        Ok(UsageRow {
            label: row.get(0)?,
            cost: row.get(1)?,
            input_tokens: row.get(2)?,
            output_tokens: row.get(3)?,
            requests: row.get(4)?,
        })
    })
    .ok()
    .map(|iter| iter.filter_map(|r| r.ok()).collect())
    .unwrap_or_default()
}

fn fetch_by_group(conn: &rusqlite::Connection, start_ts: i64) -> Vec<UsageRow> {
    let mut stmt = match conn.prepare(
        "SELECT group_alias,
                COALESCE(SUM(cost_usd), 0.0),
                COALESCE(SUM(prompt_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                COUNT(*)
         FROM requests WHERE ts >= ?1
         GROUP BY group_alias
         ORDER BY COALESCE(SUM(cost_usd), 0.0) DESC",
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    stmt.query_map([&start_ts.to_string()], |row| {
        Ok(UsageRow {
            label: row.get(0)?,
            cost: row.get(1)?,
            input_tokens: row.get(2)?,
            output_tokens: row.get(3)?,
            requests: row.get(4)?,
        })
    })
    .ok()
    .map(|iter| iter.filter_map(|r| r.ok()).collect())
    .unwrap_or_default()
}

fn fetch_by_model(conn: &rusqlite::Connection, start_ts: i64) -> Vec<UsageRow> {
    let mut stmt = match conn.prepare(
        "SELECT model_id,
                COALESCE(SUM(cost_usd), 0.0),
                COALESCE(SUM(prompt_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                COUNT(*)
         FROM requests WHERE ts >= ?1
         GROUP BY model_id
         ORDER BY COALESCE(SUM(cost_usd), 0.0) DESC",
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    stmt.query_map([&start_ts.to_string()], |row| {
        Ok(UsageRow {
            label: row.get(0)?,
            cost: row.get(1)?,
            input_tokens: row.get(2)?,
            output_tokens: row.get(3)?,
            requests: row.get(4)?,
        })
    })
    .ok()
    .map(|iter| iter.filter_map(|r| r.ok()).collect())
    .unwrap_or_default()
}

pub fn render(_app: &App, frame: &mut Frame, area: Rect) {
    ensure_polling();

    let data = DATA
        .get()
        .and_then(|m| m.lock().ok())
        .and_then(|g| g.clone());

    match data {
        Some(d) => render_usage(frame, area, &d),
        None => render_loading(frame, area),
    }
}

fn render_loading(frame: &mut Frame, area: Rect) {
    let text = vec![
        Line::from(Span::styled(
            "Usage Metrics",
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(Color::Cyan),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Loading data...",
            Style::default().fg(Color::Yellow),
        )),
    ];
    frame.render_widget(Paragraph::new(text), area);
}

fn render_usage(frame: &mut Frame, area: Rect, data: &UsageData) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(area);

    render_sub_tab_bar(frame, chunks[0], data.sub_tab, data.days_back);
    render_controls_hint(frame, chunks[1]);

    let label_col = match data.sub_tab {
        0 => "Date",
        1 => "Provider",
        2 => "Group",
        3 => "Model",
        _ => "Label",
    };

    let header = Row::new(vec![
        Cell::from(label_col).style(header_style()),
        Cell::from("Cost").style(header_style()),
        Cell::from("Input Tok").style(header_style()),
        Cell::from("Output Tok").style(header_style()),
        Cell::from("Total Tok").style(header_style()),
        Cell::from("Requests").style(header_style()),
    ])
    .bottom_margin(1);

    let mut all_rows: Vec<Row> = data
        .rows
        .iter()
        .map(|r| {
            Row::new(vec![
                Cell::from(r.label.clone()),
                Cell::from(format_cost(r.cost)),
                Cell::from(format_tokens(r.input_tokens)),
                Cell::from(format_tokens(r.output_tokens)),
                Cell::from(format_tokens(r.input_tokens + r.output_tokens)),
                Cell::from(r.requests.to_string()),
            ])
        })
        .collect();

    let totals = compute_totals(&data.rows);
    all_rows.push(
        Row::new(vec![
            Cell::from("Total").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from(format_cost(totals.cost))
                .style(Style::default().add_modifier(Modifier::BOLD).fg(Color::Yellow)),
            Cell::from(format_tokens(totals.input_tokens))
                .style(Style::default().add_modifier(Modifier::BOLD).fg(Color::Yellow)),
            Cell::from(format_tokens(totals.output_tokens))
                .style(Style::default().add_modifier(Modifier::BOLD).fg(Color::Yellow)),
            Cell::from(format_tokens(totals.input_tokens + totals.output_tokens))
                .style(Style::default().add_modifier(Modifier::BOLD).fg(Color::Yellow)),
            Cell::from(totals.requests.to_string())
                .style(Style::default().add_modifier(Modifier::BOLD).fg(Color::Yellow)),
        ])
        .top_margin(1),
    );

    let header_h = 2u16;
    let border_h = 2u16;
    let visible_h = chunks[2]
        .height
        .saturating_sub(header_h + border_h) as usize;
    let max_scroll = all_rows.len().saturating_sub(visible_h);
    let scroll = SCROLL.load(Ordering::Relaxed).min(max_scroll);
    SCROLL.store(scroll, Ordering::Relaxed);

    let visible: Vec<Row> = all_rows.into_iter().skip(scroll).take(visible_h).collect();

    let table = Table::new(
        visible,
        [
            Constraint::Min(12),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(9),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(table, chunks[2]);
}

fn render_sub_tab_bar(frame: &mut Frame, area: Rect, active: usize, days_back: u32) {
    let now = chrono::Utc::now();
    let start = (now - chrono::Duration::days(days_back as i64)).date_naive();
    let end = now.date_naive();

    let mut spans: Vec<Span> = vec![];
    for (i, name) in SUB_TABS.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        if i == active {
            spans.push(Span::styled(
                format!("[{}]", name),
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                format!(" {} ", name),
                Style::default().fg(Color::DarkGray),
            ));
        }
    }

    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        format!("{}d: {} to {}", days_back, start, end),
        Style::default().fg(Color::DarkGray),
    ));

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_controls_hint(frame: &mut Frame, area: Rect) {
    let hint = "\u{2190}\u{2192}: Switch tab   +/-: Range   r: Refresh   x: Export CSV   j/k: Scroll";
    frame.render_widget(
        Paragraph::new(Span::styled(hint, Style::default().fg(Color::DarkGray))),
        area,
    );
}

pub fn handle_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Left => {
            let cur = SUB_TAB.load(Ordering::Relaxed);
            if cur > 0 {
                SUB_TAB.store(cur - 1, Ordering::Relaxed);
                SCROLL.store(0, Ordering::Relaxed);
            }
        }
        KeyCode::Right => {
            let cur = SUB_TAB.load(Ordering::Relaxed);
            if cur < SUB_TABS.len() - 1 {
                SUB_TAB.store(cur + 1, Ordering::Relaxed);
                SCROLL.store(0, Ordering::Relaxed);
            }
        }
        KeyCode::Char('+') | KeyCode::Char('=') => {
            let cur = DAYS_BACK.load(Ordering::Relaxed);
            DAYS_BACK.store(cur + 7, Ordering::Relaxed);
            SCROLL.store(0, Ordering::Relaxed);
        }
        KeyCode::Char('-') => {
            let cur = DAYS_BACK.load(Ordering::Relaxed);
            if cur > 7 {
                DAYS_BACK.store(cur - 7, Ordering::Relaxed);
                SCROLL.store(0, Ordering::Relaxed);
            }
        }
        KeyCode::Char('r') => {
            REFRESH.store(true, Ordering::Relaxed);
        }
        KeyCode::Char('j') | KeyCode::Down => {
            let cur = SCROLL.load(Ordering::Relaxed);
            SCROLL.store(cur.saturating_add(1), Ordering::Relaxed);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let cur = SCROLL.load(Ordering::Relaxed);
            SCROLL.store(cur.saturating_sub(1), Ordering::Relaxed);
        }
        KeyCode::Char('x') => {
            if let Some(data) = DATA
                .get()
                .and_then(|m| m.lock().ok())
                .and_then(|g| g.clone())
            {
                match export_csv(&data) {
                    Ok(()) => {
                        app.toast = Some(ToastMessage::new("Exported to usage_export.csv"))
                    }
                    Err(e) => {
                        app.toast = Some(ToastMessage::new(format!("Export failed: {}", e)))
                    }
                }
            }
        }
        _ => {}
    }
}

fn compute_totals(rows: &[UsageRow]) -> UsageRow {
    rows.iter()
        .fold(
            UsageRow {
                label: String::new(),
                cost: 0.0,
                input_tokens: 0,
                output_tokens: 0,
                requests: 0,
            },
            |acc, r| UsageRow {
                label: acc.label,
                cost: acc.cost + r.cost,
                input_tokens: acc.input_tokens + r.input_tokens,
                output_tokens: acc.output_tokens + r.output_tokens,
                requests: acc.requests + r.requests,
            },
        )
}

fn export_csv(data: &UsageData) -> Result<(), std::io::Error> {
    let label_col = match data.sub_tab {
        0 => "Date",
        1 => "Provider",
        2 => "Group",
        3 => "Model",
        _ => "Label",
    };

    let mut f = std::fs::File::create("usage_export.csv")?;
    use std::io::Write;

    writeln!(f, "{},Cost,Input Tokens,Output Tokens,Total Tokens,Requests", label_col)?;

    for r in &data.rows {
        writeln!(
            f,
            "{},{:.6},{},{},{},{}",
            r.label,
            r.cost,
            r.input_tokens,
            r.output_tokens,
            r.input_tokens + r.output_tokens,
            r.requests,
        )?;
    }

    let totals = compute_totals(&data.rows);
    writeln!(
        f,
        "Total,{:.6},{},{},{},{}",
        totals.cost,
        totals.input_tokens,
        totals.output_tokens,
        totals.input_tokens + totals.output_tokens,
        totals.requests,
    )?;

    Ok(())
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

fn format_tokens(tokens: i64) -> String {
    let abs = tokens.unsigned_abs();
    if abs >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if abs >= 1_000 {
        format!("{:.1}K", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

fn header_style() -> Style {
    Style::default()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::BOLD)
}
