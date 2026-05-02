use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use crate::app::{App, ToastMessage};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState};
use tui_textarea::TextArea;

use coderouter_proxy::config::models::{FailoverConfig, Group, GroupEntry, Provider};
use coderouter_proxy::config::store;
use coderouter_proxy::proxy::router::{EntryStatusResponse, RouterStatusResponse};

static STATE: OnceLock<Mutex<Option<GroupsListState>>> = OnceLock::new();
static CACHED_STATUS: OnceLock<Mutex<Option<RouterStatusResponse>>> = OnceLock::new();
static STATUS_POLL_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static LAST_POLL_TIME: OnceLock<Mutex<Instant>> = OnceLock::new();

#[derive(Clone, Copy, PartialEq)]
enum GroupsMode {
    List,
    Detail(usize),
    AddGroupForm,
    EditGroupForm(usize),
    DeleteGroupConfirm(usize),
    AddEntryForm(usize),
    FailoverConfigEdit(usize),
}

struct GroupFormState {
    alias: TextArea<'static>,
    display_name: TextArea<'static>,
    failover: FailoverConfig,
    focused: usize,
    num_bufs: [String; 5],
}

impl GroupFormState {
    fn new_empty() -> Self {
        let fc = default_failover_config();
        Self::with_values("", "", &fc)
    }

    fn with_values(alias: &str, display_name: &str, fc: &FailoverConfig) -> Self {
        Self {
            alias: make_textarea(alias),
            display_name: make_textarea(display_name),
            failover: fc.clone(),
            focused: 0,
            num_bufs: [
                fc.consecutive_error_threshold.to_string(),
                (fc.latency_timeout_ms / 1000).to_string(),
                (fc.latency_timeout_cooldown_ms / 1000).to_string(),
                (fc.consecutive_error_cooldown_ms / 1000).to_string(),
                (fc.max_response_duration_ms / 1000).to_string(),
            ],
        }
    }

    fn is_text(&self) -> bool {
        matches!(self.focused, 0 | 1)
    }

    fn is_checkbox(&self) -> bool {
        matches!(self.focused, 2 | 3 | 4 | 6)
    }

    fn num_idx(&self) -> Option<usize> {
        match self.focused {
            5 => Some(0),
            7 => Some(1),
            8 => Some(2),
            9 => Some(3),
            10 => Some(4),
            _ => None,
        }
    }

    fn active_textarea(&mut self) -> Option<&mut TextArea<'static>> {
        match self.focused {
            0 => Some(&mut self.alias),
            1 => Some(&mut self.display_name),
            _ => None,
        }
    }

    fn next_field(&mut self) {
        self.save_current_numeric();
        self.focused = (self.focused + 1) % 11;
    }

    fn prev_field(&mut self) {
        self.save_current_numeric();
        self.focused = if self.focused == 0 { 10 } else { self.focused - 1 };
    }

    fn toggle(&mut self) {
        match self.focused {
            2 => self.failover.on_429 = !self.failover.on_429,
            3 => self.failover.on_quota_exhausted = !self.failover.on_quota_exhausted,
            4 => self.failover.on_consecutive_errors = !self.failover.on_consecutive_errors,
            6 => self.failover.on_latency_timeout = !self.failover.on_latency_timeout,
            _ => {}
        }
    }

    fn save_current_numeric(&mut self) {
        if let Some(idx) = self.num_idx() {
            match idx {
                0 => {
                    if let Ok(v) = self.num_bufs[0].parse::<u32>() {
                        self.failover.consecutive_error_threshold = v;
                    }
                }
                1 => {
                    if let Ok(v) = self.num_bufs[1].parse::<u64>() {
                        self.failover.latency_timeout_ms = v * 1000;
                    }
                }
                2 => {
                    if let Ok(v) = self.num_bufs[2].parse::<u64>() {
                        self.failover.latency_timeout_cooldown_ms = v * 1000;
                    }
                }
                3 => {
                    if let Ok(v) = self.num_bufs[3].parse::<u64>() {
                        self.failover.consecutive_error_cooldown_ms = v * 1000;
                    }
                }
                4 => {
                    if let Ok(v) = self.num_bufs[4].parse::<u64>() {
                        self.failover.max_response_duration_ms = v * 1000;
                    }
                }
                _ => {}
            }
        }
    }

    fn apply_all(&mut self) {
        self.save_current_numeric();
    }

    fn validate(&self) -> Result<(String, String), String> {
        let alias = get_ta_val(&self.alias);
        let dn = get_ta_val(&self.display_name);
        if alias.is_empty() {
            return Err("Alias is required".into());
        }
        if dn.is_empty() {
            return Err("Display name is required".into());
        }
        Ok((alias, dn))
    }
}

struct EntryFormState {
    providers: Vec<Provider>,
    prov_idx: usize,
    model_idx: usize,
    priority: TextArea<'static>,
    focused: usize,
}

impl EntryFormState {
    fn new(providers: Vec<Provider>, default_priority: u32) -> Self {
        Self {
            providers,
            prov_idx: 0,
            model_idx: 0,
            priority: make_textarea(&default_priority.to_string()),
            focused: 0,
        }
    }

    fn models(&self) -> Vec<String> {
        self.providers
            .get(self.prov_idx)
            .map(provider_model_ids)
            .unwrap_or_default()
    }

    fn is_text(&self) -> bool {
        self.focused == 2
    }

    fn active_textarea(&mut self) -> Option<&mut TextArea<'static>> {
        if self.focused == 2 {
            Some(&mut self.priority)
        } else {
            None
        }
    }
}

struct FailoverEditState {
    config: FailoverConfig,
    focused: usize,
    num_bufs: [String; 5],
}

impl FailoverEditState {
    fn new(config: FailoverConfig) -> Self {
        Self {
            num_bufs: [
                config.consecutive_error_threshold.to_string(),
                (config.latency_timeout_ms / 1000).to_string(),
                (config.latency_timeout_cooldown_ms / 1000).to_string(),
                (config.consecutive_error_cooldown_ms / 1000).to_string(),
                (config.max_response_duration_ms / 1000).to_string(),
            ],
            config,
            focused: 0,
        }
    }

    fn is_checkbox(&self) -> bool {
        matches!(self.focused, 0 | 1 | 2 | 4)
    }

    fn num_idx(&self) -> Option<usize> {
        match self.focused {
            3 => Some(0),
            5 => Some(1),
            6 => Some(2),
            7 => Some(3),
            8 => Some(4),
            _ => None,
        }
    }

    fn toggle(&mut self) {
        match self.focused {
            0 => self.config.on_429 = !self.config.on_429,
            1 => self.config.on_quota_exhausted = !self.config.on_quota_exhausted,
            2 => self.config.on_consecutive_errors = !self.config.on_consecutive_errors,
            4 => self.config.on_latency_timeout = !self.config.on_latency_timeout,
            _ => {}
        }
    }

    fn save_current_numeric(&mut self) {
        if let Some(idx) = self.num_idx() {
            match idx {
                0 => {
                    if let Ok(v) = self.num_bufs[0].parse::<u32>() {
                        self.config.consecutive_error_threshold = v;
                    }
                }
                1 => {
                    if let Ok(v) = self.num_bufs[1].parse::<u64>() {
                        self.config.latency_timeout_ms = v * 1000;
                    }
                }
                2 => {
                    if let Ok(v) = self.num_bufs[2].parse::<u64>() {
                        self.config.latency_timeout_cooldown_ms = v * 1000;
                    }
                }
                3 => {
                    if let Ok(v) = self.num_bufs[3].parse::<u64>() {
                        self.config.consecutive_error_cooldown_ms = v * 1000;
                    }
                }
                4 => {
                    if let Ok(v) = self.num_bufs[4].parse::<u64>() {
                        self.config.max_response_duration_ms = v * 1000;
                    }
                }
                _ => {}
            }
        }
    }

    fn apply_all(&mut self) {
        self.save_current_numeric();
    }
}

struct GroupsListState {
    groups: Vec<Group>,
    table_state: TableState,
    mode: GroupsMode,
    form: GroupFormState,
    entry_form: EntryFormState,
    fo_edit: FailoverEditState,
    detail_ts: TableState,
    detail_group: Option<Group>,
}

impl GroupsListState {
    fn load() -> Self {
        let groups = store::load_groups().unwrap_or_default();
        let mut table_state = TableState::default();
        if !groups.is_empty() {
            table_state.select(Some(0));
        }
        let providers = store::load_providers().unwrap_or_default();
        Self {
            groups,
            table_state,
            mode: GroupsMode::List,
            form: GroupFormState::new_empty(),
            entry_form: EntryFormState::new(providers, 1),
            fo_edit: FailoverEditState::new(default_failover_config()),
            detail_ts: TableState::default(),
            detail_group: None,
        }
    }

    fn reload(&mut self) {
        self.groups = store::load_groups().unwrap_or_default();
        let count = self.groups.len();
        if count == 0 {
            self.table_state.select(None);
        } else if let Some(sel) = self.table_state.selected() {
            if sel >= count {
                self.table_state.select(Some(count - 1));
            }
        } else {
            self.table_state.select(Some(0));
        }
    }

    fn reload_detail(&mut self) {
        if let GroupsMode::Detail(idx) | GroupsMode::AddEntryForm(idx) | GroupsMode::FailoverConfigEdit(idx) = self.mode {
            if let Some(g) = &self.detail_group {
                let gid = g.id.clone();
                if let Some(fresh) = store::load_groups().unwrap_or_default().into_iter().find(|g| g.id == gid) {
                    let mut fresh = fresh;
                    fresh.entries.sort_by_key(|e| e.priority);
                    self.detail_group = Some(fresh);
                    return;
                }
            }
            if idx < self.groups.len() {
                let gid = self.groups[idx].id.clone();
                if let Some(fresh) = store::load_groups().unwrap_or_default().into_iter().find(|g| g.id == gid) {
                    let mut fresh = fresh;
                    fresh.entries.sort_by_key(|e| e.priority);
                    self.detail_group = Some(fresh);
                }
            }
        }
    }

    fn ensure_detail_loaded(&mut self) {
        if self.detail_group.is_some() {
            return;
        }
        let idx = match self.mode {
            GroupsMode::Detail(i) | GroupsMode::AddEntryForm(i) | GroupsMode::FailoverConfigEdit(i) => i,
            _ => return,
        };
        if idx < self.groups.len() {
            let gid = self.groups[idx].id.clone();
            if let Some(g) = store::load_groups().unwrap_or_default().into_iter().find(|g| g.id == gid) {
                let mut g = g;
                g.entries.sort_by_key(|e| e.priority);
                self.detail_group = Some(g);
                self.detail_ts.select(Some(0));
            }
        }
    }

    fn refresh_providers(&mut self) {
        let providers = store::load_providers().unwrap_or_default();
        self.entry_form.providers = providers;
    }
}

fn make_textarea(initial: &str) -> TextArea<'static> {
    let mut ta = TextArea::new(vec![initial.to_string()]);
    ta.set_style(Style::default().fg(Color::White));
    ta.set_cursor_style(Style::default().fg(Color::White));
    ta
}

fn get_ta_val(ta: &TextArea<'_>) -> String {
    ta.lines()
        .iter()
        .map(|l: &String| l.as_str())
        .collect::<Vec<_>>()
        .join("")
        .trim()
        .to_string()
}

fn slugify(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn default_failover_config() -> FailoverConfig {
    FailoverConfig {
        on_429: true,
        on_quota_exhausted: true,
        on_consecutive_errors: true,
        consecutive_error_threshold: 5,
        on_latency_timeout: true,
        latency_timeout_ms: 90000,
        latency_timeout_cooldown_ms: 60000,
        consecutive_error_cooldown_ms: 600000,
        max_response_duration_ms: 1_200_000,
    }
}

fn provider_model_ids(p: &Provider) -> Vec<String> {
    let mut ids: Vec<String> = p.models.iter().map(|m| m.id.clone()).collect();
    if let Some(ov) = &p.model_overrides {
        for m in ov {
            if !ids.contains(&m.id) {
                ids.push(m.id.clone());
            }
        }
    }
    ids.sort();
    ids
}

fn ensure_loaded() {
    STATE.get_or_init(|| Mutex::new(Some(GroupsListState::load())));
}

fn poll_status() {
    if STATUS_POLL_IN_PROGRESS.load(Ordering::Relaxed) {
        return;
    }
    let should_poll = LAST_POLL_TIME
        .get()
        .and_then(|l| l.lock().ok())
        .map(|g| g.elapsed().as_secs() >= 5)
        .unwrap_or(true);

    if !should_poll {
        return;
    }

    STATUS_POLL_IN_PROGRESS.store(true, Ordering::Relaxed);
    if let Some(l) = LAST_POLL_TIME.get() {
        if let Ok(mut g) = l.lock() {
            *g = Instant::now();
        }
    }

    let status_lock = CACHED_STATUS.get_or_init(|| Mutex::new(None));
    std::thread::Builder::new()
        .name("group-status-poll".into())
        .spawn(move || {
            let result = fetch_router_status();
            if let Ok(mut g) = status_lock.lock() {
                *g = result;
            }
            STATUS_POLL_IN_PROGRESS.store(false, Ordering::Relaxed);
        })
        .ok();
}

fn fetch_router_status() -> Option<RouterStatusResponse> {
    let config = store::load_app_config().unwrap_or_default();
    let url = format!(
        "http://{}:{}/internal/router/status",
        config.proxy_host, config.proxy_port
    );
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap_or_default();

    let resp = client.get(&url).send().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: serde_json::Value = resp.json().ok()?;
    let data = body.get("data")?;
    serde_json::from_value(data.clone()).ok()
}

fn get_status_for_entry(group_alias: &str, entry_index: usize) -> Option<EntryStatusResponse> {
    let lock = CACHED_STATUS.get()?;
    let guard = lock.lock().ok()?;
    let status = guard.as_ref()?;
    status
        .entries
        .iter()
        .find(|e| e.group_alias == group_alias && e.entry_index == entry_index as u32)
        .cloned()
}

fn fmt_status(status: &str, reason: Option<&str>, cooldown_until: Option<chrono::DateTime<chrono::Utc>>) -> String {
    match status {
        "active" => "Active".to_string(),
        "cooldown" => {
            let r = reason.unwrap_or("unknown");
            let rem = cooldown_until
                .map(|until| {
                    let diff = until.signed_duration_since(chrono::Utc::now());
                    if diff.num_seconds() > 0 {
                        format!(" {}s", diff.num_seconds())
                    } else {
                        String::new()
                    }
                })
                .unwrap_or_default();
            format!("Cooldown ({}){}", r, rem)
        }
        "quotaexhausted" => "Quota Exhstd".to_string(),
        "manuallydisabled" => "Disabled".to_string(),
        _ => status.to_string(),
    }
}

fn status_color(status: &str) -> Color {
    match status {
        "active" => Color::Green,
        "cooldown" => Color::Yellow,
        "quotaexhausted" => Color::Red,
        "manuallydisabled" => Color::DarkGray,
        _ => Color::White,
    }
}

fn get_group_status_span(group: &Group) -> Span<'_> {
    let lock = match CACHED_STATUS.get() {
        Some(l) => l,
        None => {
            return Span::styled("—", Style::default().fg(Color::DarkGray));
        }
    };
    let guard = match lock.lock() {
        Ok(g) => g,
        Err(_) => return Span::styled("—", Style::default().fg(Color::DarkGray)),
    };
    let status = match guard.as_ref() {
        Some(s) => s,
        None => return Span::styled("—", Style::default().fg(Color::DarkGray)),
    };

    let mut active = 0u32;
    let mut total = 0u32;
    for (idx, _) in group.entries.iter().enumerate() {
        total += 1;
        if let Some(es) = status
            .entries
            .iter()
            .find(|e| e.group_alias == group.alias && e.entry_index == idx as u32)
        {
            if es.status == "active" {
                active += 1;
            }
        } else {
            active += 1;
        }
    }

    if total == 0 {
        Span::styled("Empty", Style::default().fg(Color::DarkGray))
    } else if active == total {
        Span::styled("Healthy", Style::default().fg(Color::Green))
    } else if active > 0 {
        Span::styled("Degraded", Style::default().fg(Color::Yellow))
    } else {
        Span::styled("Down", Style::default().fg(Color::Red))
    }
}

fn save_group_to_store(group: &Group) -> Result<(), String> {
    let mut groups = store::load_groups().unwrap_or_default();
    if let Some(pos) = groups.iter().position(|g| g.id == group.id) {
        groups[pos] = group.clone();
    } else {
        groups.push(group.clone());
    }
    store::save_groups(&groups).map_err(|e| e.to_string())
}

fn delete_group_from_store(group_id: &str) -> Result<(), String> {
    let mut groups = store::load_groups().map_err(|e| e.to_string())?;
    groups.retain(|g| g.id != group_id);
    store::save_groups(&groups).map_err(|e| e.to_string())
}

fn fo_summary(fc: &FailoverConfig) -> String {
    let checks = [
        ("429", fc.on_429),
        ("Quota", fc.on_quota_exhausted),
        ("Err", fc.on_consecutive_errors),
        ("Lat", fc.on_latency_timeout),
    ];
    let check_str: String = checks
        .iter()
        .map(|(l, v)| format!("{}:{}", l, if *v { "Y" } else { "N" }))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "{} | Thresh:{} TOut:{}s LCD:{}s ECD:{}s Max:{}s",
        check_str,
        fc.consecutive_error_threshold,
        fc.latency_timeout_ms / 1000,
        fc.latency_timeout_cooldown_ms / 1000,
        fc.consecutive_error_cooldown_ms / 1000,
        fc.max_response_duration_ms / 1000,
    )
}

pub fn render(app: &App, frame: &mut Frame, area: Rect) {
    ensure_loaded();
    CACHED_STATUS.get_or_init(|| Mutex::new(None));
    LAST_POLL_TIME.get_or_init(|| {
        Mutex::new(Instant::now() - std::time::Duration::from_secs(10))
    });

    poll_status();

    let state_ref = STATE.get();
    if state_ref.is_none() {
        render_loading(frame, area);
        return;
    }
    let state_ref = state_ref.unwrap();
    let mut guard = match state_ref.lock() {
        Ok(g) => g,
        Err(_) => {
            render_loading(frame, area);
            return;
        }
    };
    if guard.is_none() {
        render_loading(frame, area);
        return;
    }
    let state = guard.as_mut().unwrap();

    match state.mode {
        GroupsMode::Detail(_) => {
            state.ensure_detail_loaded();
            render_detail(frame, area, state, app);
        }
        GroupsMode::AddEntryForm(_) | GroupsMode::FailoverConfigEdit(_) => {
            state.ensure_detail_loaded();
            render_detail(frame, area, state, app);
            match state.mode {
                GroupsMode::AddEntryForm(_) => render_entry_form(frame, area, state),
                GroupsMode::FailoverConfigEdit(_) => render_failover_edit(frame, area, state),
                _ => {}
            }
        }
        _ => {
            render_list(frame, area, state);
            match state.mode {
                GroupsMode::List => {}
                GroupsMode::AddGroupForm | GroupsMode::EditGroupForm(_) => {
                    render_group_form(frame, area, state);
                }
                GroupsMode::DeleteGroupConfirm(_) => {
                    render_delete_confirm(frame, area, state);
                }
                _ => {}
            }
        }
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
        GroupsMode::List => handle_list_key(app, key, state),
        GroupsMode::Detail(_) => handle_detail_key(app, key, state),
        GroupsMode::AddGroupForm | GroupsMode::EditGroupForm(_) => {
            handle_group_form_key(app, key, state)
        }
        GroupsMode::DeleteGroupConfirm(_) => handle_delete_key(app, key, state),
        GroupsMode::AddEntryForm(_) => handle_entry_form_key(app, key, state),
        GroupsMode::FailoverConfigEdit(_) => handle_failover_edit_key(app, key, state),
    }
}

fn render_loading(frame: &mut Frame, area: Rect) {
    let text = vec![
        Line::from(Span::styled(
            "Routing Groups",
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(Color::Cyan),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Loading...",
            Style::default().fg(Color::Yellow),
        )),
    ];
    frame.render_widget(Paragraph::new(text), area);
}

fn render_list(frame: &mut Frame, area: Rect, state: &mut GroupsListState) {
    let header_cells = ["Alias", "Display Name", "Entries", "Active", "Status"];
    let header = Row::new(
        header_cells
            .iter()
            .map(|h| Cell::from(*h).style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD))),
    )
    .bottom_margin(1);

    let rows: Vec<Row> = state
        .groups
        .iter()
        .map(|g| {
            let total = g.entries.len();
            let active = g.entries.iter().filter(|e| e.enabled).count();
            let status_span = get_group_status_span(g);
            Row::new(vec![
                Cell::from(g.alias.as_str()).style(Style::default().fg(Color::White)),
                Cell::from(g.display_name.as_str()).style(Style::default().fg(Color::White)),
                Cell::from(total.to_string()).style(Style::default().fg(Color::DarkGray)),
                Cell::from(active.to_string()).style(Style::default().fg(Color::DarkGray)),
                Cell::from(status_span),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(20),
            Constraint::Length(20),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(10),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Span::styled(
                " Groups ",
                Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan),
            )),
    )
    .row_highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    );

    frame.render_stateful_widget(table, area, &mut state.table_state);

    let hints = if state.groups.is_empty() {
        " a:Add  ?:Help "
    } else {
        " j/k:Nav  a:Add  e:Edit  d:Del  Enter:Detail  ?:Help "
    };
    let hint = Paragraph::new(Span::styled(hints, Style::default().fg(Color::DarkGray)));
    let hint_area = Rect::new(area.x, area.y + area.height.saturating_sub(1), area.width, 1);
    frame.render_widget(hint, hint_area);
}

fn render_detail(frame: &mut Frame, area: Rect, state: &mut GroupsListState, app: &App) {
    let group = match &state.detail_group {
        Some(g) => g.clone(),
        None => {
            let msg = Paragraph::new("Group not found. Press Esc to go back.")
                .style(Style::default().fg(Color::Red));
            frame.render_widget(msg, area);
            return;
        }
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(1),
        ])
        .split(area);

    let header_lines = vec![
        Line::from(vec![
            Span::styled(
                format!(" {} ", group.alias),
                Style::default()
                    .add_modifier(Modifier::BOLD)
                    .fg(Color::Cyan),
            ),
            Span::styled(
                format!(" ({})", group.display_name),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(vec![
            Span::styled(" Entries: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                group.entries.len().to_string(),
                Style::default().fg(Color::White),
            ),
        ]),
    ];
    let header_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    frame.render_widget(Paragraph::new(header_lines).block(header_block), chunks[0]);

    let entry_header = Row::new(
        ["Pri", "Provider", "Model", "Status", "On"]
            .iter()
            .map(|h| {
                Cell::from(*h)
                    .style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD))
            }),
    )
    .bottom_margin(1);

    let alias = group.alias.clone();
    let rows: Vec<Row> = group
        .entries
        .iter()
        .enumerate()
        .map(|(idx, e)| {
            let (status_text, status_col) = if let Some(es) = get_status_for_entry(&alias, idx) {
                (
                    fmt_status(&es.status, es.cooldown_reason.as_deref(), es.cooldown_until),
                    status_color(&es.status),
                )
            } else {
                let (s, c) = if !e.enabled {
                    ("Disabled".to_string(), Color::DarkGray)
                } else {
                    ("Active".to_string(), Color::Green)
                };
                (s, c)
            };
            Row::new(vec![
                Cell::from(e.priority.to_string()).style(Style::default().fg(Color::Yellow)),
                Cell::from(e.provider_id.as_str()).style(Style::default().fg(Color::White)),
                Cell::from(e.model_id.as_str()).style(Style::default().fg(Color::White)),
                Cell::from(status_text).style(Style::default().fg(status_col)),
                Cell::from(if e.enabled { "Yes" } else { "No" })
                    .style(Style::default().fg(if e.enabled { Color::Green } else { Color::DarkGray })),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(4),
            Constraint::Length(16),
            Constraint::Min(16),
            Constraint::Length(24),
            Constraint::Length(4),
        ],
    )
    .header(entry_header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Span::styled(
                " Entries ",
                Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan),
            )),
    )
    .row_highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    );

    frame.render_stateful_widget(table, chunks[1], &mut state.detail_ts);

    let fo_text = fo_summary(&group.failover_config);
    let fo_para = Paragraph::new(Line::from(vec![
        Span::styled(" Failover: ", Style::default().fg(Color::DarkGray)),
        Span::styled(fo_text, Style::default().fg(Color::White)),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(fo_para, chunks[2]);

    let status_text = if !app.proxy_running {
        " Proxy offline — live status unavailable ".to_string()
    } else if let Some(lock) = CACHED_STATUS.get() {
        if let Ok(guard) = lock.lock() {
            if guard.is_some() {
                let entries: Vec<String> = group
                    .entries
                    .iter()
                    .enumerate()
                    .map(|(idx, e)| {
                        if let Some(es) = get_status_for_entry(&alias, idx) {
                            format!("{}:{}", e.model_id, &es.status[..1].to_uppercase())
                        } else {
                            format!("{}:?", e.model_id)
                        }
                    })
                    .collect();
                format!(" {} ", entries.join(" | "))
            } else {
                " Waiting for status... ".to_string()
            }
        } else {
            " Status unavailable ".to_string()
        }
    } else {
        " Waiting for status... ".to_string()
    };

    let status_para = Paragraph::new(Span::styled(
        status_text,
        Style::default().fg(Color::DarkGray),
    ))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Span::styled(" Live ", Style::default().fg(Color::DarkGray))),
    );
    frame.render_widget(status_para, chunks[3]);

    let hints = " j/k:Nav  ↑/↓:Reorder  e:Toggle  d:Del  a:Add  f:Config  Esc:Back ";
    let hint = Paragraph::new(Span::styled(hints, Style::default().fg(Color::DarkGray)));
    frame.render_widget(hint, chunks[4]);
}

fn render_group_form(frame: &mut Frame, area: Rect, state: &mut GroupsListState) {
    let is_add = matches!(state.mode, GroupsMode::AddGroupForm);
    let title = if is_add { " Add Group " } else { " Edit Group " };

    let form_width = 64.min(area.width);
    let form_height = 22.min(area.height);
    let x = (area.width.saturating_sub(form_width)) / 2;
    let y = (area.height.saturating_sub(form_height)) / 2;
    let popup_area = Rect::new(x, y, form_width, form_height);

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(title)
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner);

    let focused = state.form.focused;

    let alias_style = if focused == 0 {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    state.form.alias.set_block(
        Block::default()
            .title(" Alias ")
            .borders(Borders::ALL)
            .border_style(alias_style),
    );
    frame.render_widget(&state.form.alias, layout[0]);

    let dn_style = if focused == 1 {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    state.form.display_name.set_block(
        Block::default()
            .title(" Display Name ")
            .borders(Borders::ALL)
            .border_style(dn_style),
    );
    frame.render_widget(&state.form.display_name, layout[1]);

    let fo_header = Paragraph::new(Span::styled(
        " Failover Config ",
        Style::default()
            .add_modifier(Modifier::BOLD)
            .fg(Color::Cyan),
    ));
    frame.render_widget(fo_header, layout[2]);

    let fc = &state.form.failover;
    let fields: [(bool, &str); 4] = [
        (fc.on_429, "On HTTP 429"),
        (fc.on_quota_exhausted, "On Quota Exhausted"),
        (fc.on_consecutive_errors, "On Consecutive Errors"),
        (fc.on_latency_timeout, "On Latency Timeout"),
    ];
    let fo_field_indices = [2, 3, 4, 6];

    for (i, (checked, label)) in fields.iter().enumerate() {
        let fi = fo_field_indices[i];
        let line_focused = focused == fi;
        let check = if *checked { "[x]" } else { "[ ]" };
        let style = if line_focused {
            Style::default().fg(Color::White).bg(Color::DarkGray)
        } else {
            Style::default().fg(Color::White)
        };
        let p = Paragraph::new(Line::from(Span::styled(
            format!(" {} {} ", check, label),
            style,
        )));
        frame.render_widget(p, layout[3 + i]);
    }

    let num_fields: [(usize, &str, &str); 5] = [
        (5, "Error Threshold", &state.form.num_bufs[0]),
        (7, "Timeout (s)", &state.form.num_bufs[1]),
        (8, "Lat Cooldown (s)", &state.form.num_bufs[2]),
        (9, "Err Cooldown (s)", &state.form.num_bufs[3]),
        (10, "Max Duration (s)", &state.form.num_bufs[4]),
    ];

    for (i, (fi, label, val)) in num_fields.iter().enumerate() {
        let line_focused = focused == *fi;
        let cursor = if line_focused { ">" } else { " " };
        let style = if line_focused {
            Style::default().fg(Color::White).bg(Color::DarkGray)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let p = Paragraph::new(Line::from(vec![
            Span::styled(format!(" {} {}: ", cursor, label), style),
            Span::styled(val.to_string(), Style::default().fg(Color::White)),
        ]));
        frame.render_widget(p, layout[7 + i]);
    }

    let hint_text = " Tab:Next  Space:Toggle  Enter:Save  Esc:Cancel ";
    let hint = Paragraph::new(Span::styled(
        hint_text,
        Style::default().fg(Color::DarkGray),
    ))
    .alignment(Alignment::Center);
    frame.render_widget(hint, layout[12]);
}

fn render_delete_confirm(frame: &mut Frame, area: Rect, state: &GroupsListState) {
    let idx = match state.mode {
        GroupsMode::DeleteGroupConfirm(i) => i,
        _ => return,
    };
    let name = state
        .groups
        .get(idx)
        .map(|g| g.alias.as_str())
        .unwrap_or("?");

    let msg = format!("Delete group '{}'?", name);
    let width = (msg.len() as u16 + 6).min(area.width);
    let height = 5.min(area.height);
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup_area = Rect::new(x, y, width, height);

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" Confirm Delete ")
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let lines = vec![
        Paragraph::new(Span::styled(msg, Style::default().fg(Color::White)))
            .alignment(Alignment::Center),
        Paragraph::new(""),
        Paragraph::new(Span::styled(
            " y:Confirm  Esc:Cancel ",
            Style::default().fg(Color::DarkGray),
        ))
        .alignment(Alignment::Center),
    ];

    let text_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);

    for (i, p) in lines.into_iter().enumerate() {
        if i < text_layout.len() {
            frame.render_widget(p, text_layout[i]);
        }
    }
}

fn render_entry_form(frame: &mut Frame, area: Rect, state: &mut GroupsListState) {
    let form_width = 56.min(area.width);
    let form_height = 14.min(area.height);
    let x = (area.width.saturating_sub(form_width)) / 2;
    let y = (area.height.saturating_sub(form_height)) / 2;
    let popup_area = Rect::new(x, y, form_width, form_height);

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" Add Entry ")
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .split(inner);

    let ef = &state.entry_form;
    let focused = ef.focused;

    let providers: Vec<&str> = ef.providers.iter().map(|p| p.id.as_str()).collect();
    let models = ef.models();

    let prov_display = providers.get(ef.prov_idx).unwrap_or(&"<none>");
    let prov_style = if focused == 0 {
        Style::default().fg(Color::White).bg(Color::DarkGray)
    } else {
        Style::default().fg(Color::White)
    };
    let prov_p = Paragraph::new(Line::from(vec![
        Span::styled(" Provider: ", Style::default().fg(if focused == 0 { Color::Cyan } else { Color::DarkGray })),
        Span::styled(format!("{} [{}/{}]", prov_display, ef.prov_idx + 1, providers.len()), prov_style),
    ]));
    frame.render_widget(prov_p, layout[0]);

    let none_model = "<none>".to_string();
    let model_display = models.get(ef.model_idx).unwrap_or(&none_model);
    let model_style = if focused == 1 {
        Style::default().fg(Color::White).bg(Color::DarkGray)
    } else {
        Style::default().fg(Color::White)
    };
    let model_p = Paragraph::new(Line::from(vec![
        Span::styled(" Model: ", Style::default().fg(if focused == 1 { Color::Cyan } else { Color::DarkGray })),
        Span::styled(format!("{} [{}/{}]", model_display, ef.model_idx + 1, models.len()), model_style),
    ]));
    frame.render_widget(model_p, layout[1]);

    let hint_spans = if focused <= 1 {
        Span::styled(" j/k:Select  Tab:Next  Esc:Cancel ", Style::default().fg(Color::DarkGray))
    } else {
        Span::styled(" Tab:Next  Enter:Save  Esc:Cancel ", Style::default().fg(Color::DarkGray))
    };
    frame.render_widget(Paragraph::new(Line::from(hint_spans)).alignment(Alignment::Center), layout[2]);

    frame.render_widget(Paragraph::new(""), layout[3]);

    let prio_style = if focused == 2 {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    state.entry_form.priority.set_block(
        Block::default()
            .title(" Priority ")
            .borders(Borders::ALL)
            .border_style(prio_style),
    );
    frame.render_widget(&state.entry_form.priority, layout[4]);
}

fn render_failover_edit(frame: &mut Frame, area: Rect, state: &mut GroupsListState) {
    let form_width = 56.min(area.width);
    let form_height = 15.min(area.height);
    let x = (area.width.saturating_sub(form_width)) / 2;
    let y = (area.height.saturating_sub(form_height)) / 2;
    let popup_area = Rect::new(x, y, form_width, form_height);

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" Failover Config ")
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner);

    let fe = &state.fo_edit;
    let focused = fe.focused;
    let fc = &fe.config;

    let checkboxes: [(bool, &str); 4] = [
        (fc.on_429, "On HTTP 429"),
        (fc.on_quota_exhausted, "On Quota Exhausted"),
        (fc.on_consecutive_errors, "On Consecutive Errors"),
        (fc.on_latency_timeout, "On Latency Timeout"),
    ];
    let cb_indices = [0, 1, 2, 4];

    for (i, (checked, label)) in checkboxes.iter().enumerate() {
        let fi = cb_indices[i];
        let line_focused = focused == fi;
        let check = if *checked { "[x]" } else { "[ ]" };
        let style = if line_focused {
            Style::default().fg(Color::White).bg(Color::DarkGray)
        } else {
            Style::default().fg(Color::White)
        };
        let p = Paragraph::new(Line::from(Span::styled(
            format!(" {} {} ", check, label),
            style,
        )));
        frame.render_widget(p, layout[i]);
    }

    let num_fields: [(usize, &str, &str); 5] = [
        (3, "Threshold", &fe.num_bufs[0]),
        (5, "Timeout (s)", &fe.num_bufs[1]),
        (6, "Lat Cooldown (s)", &fe.num_bufs[2]),
        (7, "Err Cooldown (s)", &fe.num_bufs[3]),
        (8, "Max Duration (s)", &fe.num_bufs[4]),
    ];

    for (i, (fi, label, val)) in num_fields.iter().enumerate() {
        let line_focused = focused == *fi;
        let cursor = if line_focused { ">" } else { " " };
        let style = if line_focused {
            Style::default().fg(Color::White).bg(Color::DarkGray)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let p = Paragraph::new(Line::from(vec![
            Span::styled(format!(" {} {}: ", cursor, label), style),
            Span::styled(val.to_string(), Style::default().fg(Color::White)),
        ]));
        frame.render_widget(p, layout[4 + i]);
    }

    let hint_text = " Tab:Next  Space:Toggle  Enter:Save  Esc:Cancel ";
    let hint = Paragraph::new(Span::styled(
        hint_text,
        Style::default().fg(Color::DarkGray),
    ))
    .alignment(Alignment::Center);
    frame.render_widget(hint, layout[9]);
}

pub fn is_form_active() -> bool {
    if let Some(state_ref) = STATE.get() {
        if let Ok(guard) = state_ref.lock() {
            if let Some(state) = guard.as_ref() {
                return matches!(
                    state.mode,
                    GroupsMode::AddGroupForm
                        | GroupsMode::EditGroupForm(_)
                        | GroupsMode::AddEntryForm(_)
                        | GroupsMode::FailoverConfigEdit(_)
                );
            }
        }
    }
    false
}

fn handle_list_key(_app: &mut App, key: KeyEvent, state: &mut GroupsListState) {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if !state.groups.is_empty() {
                let i = state.table_state.selected().unwrap_or(0);
                let next = (i + 1).min(state.groups.len() - 1);
                state.table_state.select(Some(next));
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if !state.groups.is_empty() {
                let i = state.table_state.selected().unwrap_or(0);
                let prev = i.saturating_sub(1);
                state.table_state.select(Some(prev));
            }
        }
        KeyCode::Enter => {
            if let Some(i) = state.table_state.selected() {
                if i < state.groups.len() {
                    state.mode = GroupsMode::Detail(i);
                    state.detail_group = None;
                    state.detail_ts.select(Some(0));
                }
            }
        }
        KeyCode::Char('a') => {
            state.form = GroupFormState::new_empty();
            state.mode = GroupsMode::AddGroupForm;
        }
        KeyCode::Char('e') => {
            if let Some(i) = state.table_state.selected() {
                if i < state.groups.len() {
                    let g = &state.groups[i];
                    state.form = GroupFormState::with_values(
                        &g.alias,
                        &g.display_name,
                        &g.failover_config,
                    );
                    state.mode = GroupsMode::EditGroupForm(i);
                }
            }
        }
        KeyCode::Char('d') => {
            if let Some(i) = state.table_state.selected() {
                if i < state.groups.len() {
                    state.mode = GroupsMode::DeleteGroupConfirm(i);
                }
            }
        }
        _ => {}
    }
}

fn handle_detail_key(app: &mut App, key: KeyEvent, state: &mut GroupsListState) {
    let group = match &mut state.detail_group {
        Some(g) => g,
        None => return,
    };
    let entry_count = group.entries.len();

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if entry_count > 0 {
                let i = state.detail_ts.selected().unwrap_or(0);
                let next = (i + 1).min(entry_count - 1);
                state.detail_ts.select(Some(next));
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if entry_count > 0 {
                let i = state.detail_ts.selected().unwrap_or(0);
                let prev = i.saturating_sub(1);
                state.detail_ts.select(Some(prev));
            }
        }
        KeyCode::Char('J') => {
            let sel = state.detail_ts.selected().unwrap_or(0);
            if sel + 1 < entry_count {
                group.entries.sort_by_key(|e| e.priority);
                group.entries.swap(sel, sel + 1);
                for (i, e) in group.entries.iter_mut().enumerate() {
                    e.priority = (i + 1) as u32;
                }
                state.detail_ts.select(Some(sel + 1));
                let _ = save_group_to_store(group);
                app.toast = Some(ToastMessage::new("Entry moved down"));
            }
        }
        KeyCode::Char('K') => {
            let sel = state.detail_ts.selected().unwrap_or(0);
            if sel > 0 && entry_count > 1 {
                group.entries.sort_by_key(|e| e.priority);
                group.entries.swap(sel, sel - 1);
                for (i, e) in group.entries.iter_mut().enumerate() {
                    e.priority = (i + 1) as u32;
                }
                state.detail_ts.select(Some(sel - 1));
                let _ = save_group_to_store(group);
                app.toast = Some(ToastMessage::new("Entry moved up"));
            }
        }
        KeyCode::Char('e') => {
            let sel = state.detail_ts.selected().unwrap_or(0);
            if sel < entry_count {
                group.entries[sel].enabled = !group.entries[sel].enabled;
                let _ = save_group_to_store(group);
                let label = if group.entries[sel].enabled {
                    "enabled"
                } else {
                    "disabled"
                };
                app.toast = Some(ToastMessage::new(format!(
                    "Entry {}",
                    label
                )));
            }
        }
        KeyCode::Char('d') => {
            let sel = state.detail_ts.selected().unwrap_or(0);
            if sel < entry_count {
                group.entries.remove(sel);
                for (i, e) in group.entries.iter_mut().enumerate() {
                    e.priority = (i + 1) as u32;
                }
                let _ = save_group_to_store(group);
                if entry_count > 1 {
                    state.detail_ts.select(Some(sel.min(entry_count - 2)));
                }
                app.toast = Some(ToastMessage::new("Entry deleted"));
                state.reload_detail();
            }
        }
        KeyCode::Char('a') => {
            let max_pri = group.entries.iter().map(|e| e.priority).max().unwrap_or(0);
            state.refresh_providers();
            state.entry_form = EntryFormState::new(state.entry_form.providers.clone(), max_pri + 1);
            let idx = match state.mode {
                GroupsMode::Detail(i) => i,
                _ => 0,
            };
            state.mode = GroupsMode::AddEntryForm(idx);
        }
        KeyCode::Char('f') => {
            let fc = group.failover_config.clone();
            state.fo_edit = FailoverEditState::new(fc);
            let idx = match state.mode {
                GroupsMode::Detail(i) => i,
                _ => 0,
            };
            state.mode = GroupsMode::FailoverConfigEdit(idx);
        }
        KeyCode::Esc => {
            state.mode = GroupsMode::List;
            state.detail_group = None;
            state.reload();
        }
        _ => {}
    }
}

fn handle_group_form_key(app: &mut App, key: KeyEvent, state: &mut GroupsListState) {
    match key.code {
        KeyCode::Tab => {
            state.form.next_field();
        }
        KeyCode::BackTab => {
            state.form.prev_field();
        }
        KeyCode::Enter => {
            submit_group_form(app, state);
        }
        KeyCode::Esc => {
            state.mode = GroupsMode::List;
        }
        KeyCode::Char(' ') if state.form.is_checkbox() => {
            state.form.toggle();
        }
        _ => {
            if state.form.is_text() {
                if let Some(ta) = state.form.active_textarea() {
                    ta.input(key);
                }
            } else if let Some(idx) = state.form.num_idx() {
                match key.code {
                    KeyCode::Char(c) if c.is_ascii_digit() => {
                        state.form.num_bufs[idx].push(c);
                    }
                    KeyCode::Backspace => {
                        state.form.num_bufs[idx].pop();
                    }
                    _ => {}
                }
            }
        }
    }
}

fn submit_group_form(app: &mut App, state: &mut GroupsListState) {
    state.form.apply_all();
    match state.form.validate() {
        Ok((alias, display_name)) => {
            let fc = state.form.failover.clone();
            let result = match state.mode {
                GroupsMode::AddGroupForm => {
                    let id = slugify(&alias);
                    let duplicate = store::load_groups()
                        .unwrap_or_default()
                        .iter()
                        .any(|g| g.id == id);
                    if duplicate {
                        Err(format!("Group '{}' already exists", id))
                    } else {
                        let group = Group {
                            id,
                            alias,
                            display_name,
                            entries: vec![],
                            failover_config: fc,
                        };
                        save_group_to_store(&group)
                    }
                }
                GroupsMode::EditGroupForm(idx) => {
                    if idx < state.groups.len() {
                        let gid = state.groups[idx].id.clone();
                        let mut groups = store::load_groups().unwrap_or_default();
                        if let Some(g) = groups.iter_mut().find(|g| g.id == gid) {
                            g.alias = alias;
                            g.display_name = display_name;
                            g.failover_config = fc;
                        }
                        store::save_groups(&groups).map_err(|e| e.to_string())
                    } else {
                        Err("Invalid selection".into())
                    }
                }
                _ => Ok(()),
            };

            match result {
                Ok(()) => {
                    app.toast = Some(ToastMessage::new("Group saved"));
                    state.reload();
                    state.mode = GroupsMode::List;
                }
                Err(e) => {
                    app.toast = Some(ToastMessage::new(format!("Error: {}", e)));
                }
            }
        }
        Err(e) => {
            app.toast = Some(ToastMessage::new(e));
        }
    }
}

fn handle_delete_key(app: &mut App, key: KeyEvent, state: &mut GroupsListState) {
    match key.code {
        KeyCode::Char('y') => {
            let idx = match state.mode {
                GroupsMode::DeleteGroupConfirm(i) => i,
                _ => return,
            };
            if idx < state.groups.len() {
                let alias = state.groups[idx].alias.clone();
                let gid = state.groups[idx].id.clone();
                match delete_group_from_store(&gid) {
                    Ok(()) => {
                        app.toast = Some(ToastMessage::new(format!("Deleted '{}'", alias)));
                        state.reload();
                    }
                    Err(e) => {
                        app.toast = Some(ToastMessage::new(format!("Error: {}", e)));
                    }
                }
            }
            state.mode = GroupsMode::List;
        }
        KeyCode::Esc => {
            state.mode = GroupsMode::List;
        }
        _ => {}
    }
}

fn handle_entry_form_key(app: &mut App, key: KeyEvent, state: &mut GroupsListState) {
    let ef = &mut state.entry_form;

    match key.code {
        KeyCode::Tab => {
            ef.focused = (ef.focused + 1) % 3;
        }
        KeyCode::BackTab => {
            ef.focused = if ef.focused == 0 { 2 } else { ef.focused - 1 };
        }
        KeyCode::Enter => {
            submit_entry_form(app, state);
        }
        KeyCode::Esc => {
            let idx = match state.mode {
                GroupsMode::AddEntryForm(i) => i,
                _ => 0,
            };
            state.mode = GroupsMode::Detail(idx);
        }
        KeyCode::Char('j') | KeyCode::Down if ef.focused == 0 => {
            if !ef.providers.is_empty() {
                ef.prov_idx = (ef.prov_idx + 1).min(ef.providers.len() - 1);
                ef.model_idx = 0;
            }
        }
        KeyCode::Char('k') | KeyCode::Up if ef.focused == 0 => {
            ef.prov_idx = ef.prov_idx.saturating_sub(1);
            ef.model_idx = 0;
        }
        KeyCode::Char('j') | KeyCode::Down if ef.focused == 1 => {
            let models = ef.models();
            if !models.is_empty() {
                ef.model_idx = (ef.model_idx + 1).min(models.len() - 1);
            }
        }
        KeyCode::Char('k') | KeyCode::Up if ef.focused == 1 => {
            ef.model_idx = ef.model_idx.saturating_sub(1);
        }
        _ => {
            if ef.focused == 2 {
                ef.priority.input(key);
            }
        }
    }
}

fn submit_entry_form(app: &mut App, state: &mut GroupsListState) {
    let ef = &state.entry_form;

    let provider_id = match ef.providers.get(ef.prov_idx) {
        Some(p) => p.id.clone(),
        None => {
            app.toast = Some(ToastMessage::new("No provider selected"));
            return;
        }
    };

    let models = ef.models();
    let model_id = match models.get(ef.model_idx) {
        Some(m) => m.clone(),
        None => {
            app.toast = Some(ToastMessage::new("No model selected"));
            return;
        }
    };

    let priority_str = get_ta_val(&ef.priority);
    let priority: u32 = match priority_str.parse() {
        Ok(v) if v > 0 => v,
        _ => {
            app.toast = Some(ToastMessage::new("Invalid priority (must be > 0)"));
            return;
        }
    };

    let group = match &mut state.detail_group {
        Some(g) => g,
        None => {
            app.toast = Some(ToastMessage::new("No group loaded"));
            return;
        }
    };

    group.entries.push(GroupEntry {
        provider_id,
        model_id,
        priority,
        daily_token_quota_override: None,
        enabled: true,
        status: "active".to_string(),
        cooldown_until: None,
    });
    group.entries.sort_by_key(|e| e.priority);

    match save_group_to_store(group) {
        Ok(()) => {
            app.toast = Some(ToastMessage::new("Entry added"));
            let idx = match state.mode {
                GroupsMode::AddEntryForm(i) => i,
                _ => 0,
            };
            state.mode = GroupsMode::Detail(idx);
            state.reload_detail();
        }
        Err(e) => {
            app.toast = Some(ToastMessage::new(format!("Error: {}", e)));
        }
    }
}

fn handle_failover_edit_key(app: &mut App, key: KeyEvent, state: &mut GroupsListState) {
    let fe = &mut state.fo_edit;

    match key.code {
        KeyCode::Tab => {
            fe.save_current_numeric();
            fe.focused = (fe.focused + 1) % 9;
        }
        KeyCode::BackTab => {
            fe.save_current_numeric();
            fe.focused = if fe.focused == 0 { 8 } else { fe.focused - 1 };
        }
        KeyCode::Enter => {
            fe.apply_all();
            let fc = fe.config.clone();
            let group = match &mut state.detail_group {
                Some(g) => g,
                None => {
                    state.mode = GroupsMode::List;
                    return;
                }
            };
            group.failover_config = fc;
            match save_group_to_store(group) {
                Ok(()) => {
                    app.toast = Some(ToastMessage::new("Failover config saved"));
                    let idx = match state.mode {
                        GroupsMode::FailoverConfigEdit(i) => i,
                        _ => 0,
                    };
                    state.mode = GroupsMode::Detail(idx);
                    state.reload_detail();
                }
                Err(e) => {
                    app.toast = Some(ToastMessage::new(format!("Error: {}", e)));
                }
            }
        }
        KeyCode::Esc => {
            let idx = match state.mode {
                GroupsMode::FailoverConfigEdit(i) => i,
                _ => 0,
            };
            state.mode = GroupsMode::Detail(idx);
        }
        KeyCode::Char(' ') if fe.is_checkbox() => {
            fe.toggle();
        }
        _ => {
            if let Some(idx) = fe.num_idx() {
                match key.code {
                    KeyCode::Char(c) if c.is_ascii_digit() => {
                        fe.num_bufs[idx].push(c);
                    }
                    KeyCode::Backspace => {
                        fe.num_bufs[idx].pop();
                    }
                    _ => {}
                }
            }
        }
    }
}
