use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::app::{App, ToastMessage};
use crate::presets;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState};
use tui_textarea::TextArea;

use coderouter_proxy::config::models::{Provider, ProviderModel};
use coderouter_proxy::config::store;
use coderouter_proxy::credentials::keychain;

static TEST_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static REFRESH_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static TEST_RESULT: OnceLock<Mutex<Option<(String, bool)>>> = OnceLock::new();
static REFRESH_RESULT: OnceLock<Mutex<Option<(String, bool)>>> = OnceLock::new();

#[derive(Clone)]
struct ProviderRow {
    id: String,
    name: String,
    protocol: String,
    base_url: String,
    enabled: bool,
    model_count: usize,
    has_credential: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum ProviderMode {
    List,
    AddForm,
    EditForm(usize),
    DeleteConfirm(usize),
    DetailView(usize),
}

struct ProviderFormState {
    name: TextArea<'static>,
    base_url: TextArea<'static>,
    protocol: TextArea<'static>,
    api_key: TextArea<'static>,
    focused: usize,
}

impl ProviderFormState {
    fn new_empty() -> Self {
        Self::with_values("", "", "openai", "")
    }

    fn with_values(name: &str, base_url: &str, protocol: &str, api_key: &str) -> Self {
        let name = make_textarea(name);
        let base_url = make_textarea(base_url);
        let protocol = make_textarea(protocol);
        let mut api_key = make_textarea(api_key);
        api_key.set_mask_char('\u{25cf}');
        Self {
            name,
            base_url,
            protocol,
            api_key,
            focused: 0,
        }
    }

    fn active_textarea(&mut self) -> &mut TextArea<'static> {
        match self.focused {
            0 => &mut self.name,
            1 => &mut self.base_url,
            2 => &mut self.protocol,
            _ => &mut self.api_key,
        }
    }

    fn get_value(textarea: &TextArea<'_>) -> String {
        textarea.lines().iter().map(|l: &String| l.as_str()).collect::<Vec<_>>().join("").trim().to_string()
    }

    fn validate(&self) -> Result<(String, String, String, String), String> {
        let name = Self::get_value(&self.name);
        let base_url = Self::get_value(&self.base_url);
        let protocol = Self::get_value(&self.protocol);
        let api_key = Self::get_value(&self.api_key);

        if name.is_empty() {
            return Err("Name is required".into());
        }
        if base_url.is_empty() {
            return Err("Base URL is required".into());
        }
        if api_key.is_empty() {
            return Err("API Key is required".into());
        }
        let protocol = if protocol.is_empty() {
            "openai".to_string()
        } else {
            protocol
        };
        Ok((name, base_url, protocol, api_key))
    }
}

#[derive(Clone)]
struct OverrideFormState {
    model_id: String,
    context_window: String,
    max_output: String,
    input_cost: String,
    output_cost: String,
    protocol: String,
    focused: usize,
}

impl OverrideFormState {
    fn new() -> Self {
        Self {
            model_id: String::new(),
            context_window: String::new(),
            max_output: String::new(),
            input_cost: String::new(),
            output_cost: String::new(),
            protocol: String::new(),
            focused: 0,
        }
    }

    fn active_field(&mut self) -> &mut String {
        match self.focused {
            0 => &mut self.model_id,
            1 => &mut self.context_window,
            2 => &mut self.max_output,
            3 => &mut self.input_cost,
            4 => &mut self.output_cost,
            _ => &mut self.protocol,
        }
    }

    fn next_field(&mut self) {
        self.focused = (self.focused + 1) % 6;
    }

    fn prev_field(&mut self) {
        self.focused = if self.focused == 0 { 5 } else { self.focused - 1 };
    }

    fn to_provider_model(&self) -> Option<ProviderModel> {
        let id = self.model_id.trim().to_string();
        if id.is_empty() {
            return None;
        }
        Some(ProviderModel {
            id,
            context_window: self.context_window.trim().parse().ok(),
            max_output_tokens: self.max_output.trim().parse().ok(),
            input_cost_per_1m: self.input_cost.trim().parse().ok(),
            output_cost_per_1m: self.output_cost.trim().parse().ok(),
            last_refreshed: None,
            protocol: if self.protocol.trim().is_empty() {
                None
            } else {
                Some(self.protocol.trim().to_string())
            },
        })
    }

    fn clear(&mut self) {
        self.model_id.clear();
        self.context_window.clear();
        self.max_output.clear();
        self.input_cost.clear();
        self.output_cost.clear();
        self.protocol.clear();
        self.focused = 0;
    }
}

fn make_textarea(initial: &str) -> TextArea<'static> {
    let mut ta = TextArea::new(vec![initial.to_string()]);
    ta.set_style(Style::default().fg(Color::White));
    ta.set_cursor_style(Style::default().fg(Color::White));
    ta
}

struct ProviderListState {
    providers: Vec<ProviderRow>,
    table_state: TableState,
    mode: ProviderMode,
    form: ProviderFormState,
    detail_provider: Option<Provider>,
    detail_scroll: usize,
    preset_active: bool,
    preset_index: usize,
    model_overrides: Vec<ProviderModel>,
    show_overrides: bool,
    override_selected: usize,
    adding_override: bool,
    override_focused: bool,
    override_form: OverrideFormState,
}

static STATE: OnceLock<Mutex<Option<ProviderListState>>> = OnceLock::new();

fn tokio_runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Runtime::new().expect("Failed to create tokio runtime"))
}

fn blocking_store_credential(provider_id: &str, api_key: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tokio_runtime().block_on(keychain::store_credential(provider_id, api_key))
}

fn blocking_get_credential(provider_id: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    tokio_runtime().block_on(keychain::get_credential(provider_id))
}

fn blocking_delete_credential(provider_id: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tokio_runtime().block_on(keychain::delete_credential(provider_id))
}

fn notify_sidecar_reload() {
    let config = store::load_app_config().unwrap_or_default();
    let url = format!("http://{}:{}/internal/config/reload", config.proxy_host, config.proxy_port);
    if let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        let _ = client.post(&url).send();
    }
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

fn truncate_url(url: &str, max_len: usize) -> String {
    if url.len() <= max_len {
        url.to_string()
    } else {
        format!("{}...", &url[..max_len.saturating_sub(3)])
    }
}

fn format_tokens(val: Option<u64>) -> String {
    match val {
        None => "—".to_string(),
        Some(v) if v >= 1_000_000 => format!("{:.1}M", v as f64 / 1_000_000.0),
        Some(v) if v >= 1_000 => format!("{}K", v / 1_000),
        Some(v) => v.to_string(),
    }
}

fn format_cost_per_m(val: Option<f64>) -> String {
    match val {
        None => "—".to_string(),
        Some(v) if v < 0.01 => format!("${:.4}", v),
        Some(v) => format!("${:.2}", v),
    }
}

fn load_full_provider(provider_id: &str) -> Option<Provider> {
    store::load_providers()
        .unwrap_or_default()
        .into_iter()
        .find(|p| p.id == provider_id)
}

fn load_provider_rows() -> Vec<ProviderRow> {
    let providers = store::load_providers().unwrap_or_default();
    providers
        .iter()
        .map(|p| {
            let model_count = p.models.len() + p.model_overrides.as_ref().map_or(0, |v| v.len());
            let has_credential = blocking_get_credential(&p.credential_key).is_ok();
            ProviderRow {
                id: p.id.clone(),
                name: p.name.clone(),
                protocol: p.protocol.clone(),
                base_url: p.base_url.clone(),
                enabled: p.enabled,
                model_count,
                has_credential,
            }
        })
        .collect()
}

impl ProviderListState {
    fn load() -> Self {
        let providers = load_provider_rows();
        let mut table_state = TableState::default();
        if !providers.is_empty() {
            table_state.select(Some(0));
        }
        Self {
            providers,
            table_state,
            mode: ProviderMode::List,
            form: ProviderFormState::new_empty(),
            detail_provider: None,
            detail_scroll: 0,
            preset_active: false,
            preset_index: 0,
            model_overrides: Vec::new(),
            show_overrides: false,
            override_selected: 0,
            adding_override: false,
            override_focused: false,
            override_form: OverrideFormState::new(),
        }
    }

    fn reload(&mut self) {
        self.providers = load_provider_rows();
        let count = self.providers.len();
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
}

fn ensure_loaded() {
    STATE.get_or_init(|| Mutex::new(Some(ProviderListState::load())));
}

fn check_pending_results(app: &mut App) {
    if let Some(r) = TEST_RESULT.get() {
        if let Ok(mut g) = r.lock() {
            if let Some((msg, success)) = g.take() {
                let toast_msg = if success {
                    format!("✓ {}", msg)
                } else {
                    format!("✗ {}", msg)
                };
                app.toast = Some(ToastMessage::new(toast_msg));
            }
        }
    }
    if let Some(r) = REFRESH_RESULT.get() {
        if let Ok(mut g) = r.lock() {
            if let Some((msg, success)) = g.take() {
                let toast_msg = if success {
                    format!("✓ {}", msg)
                } else {
                    format!("✗ {}", msg)
                };
                app.toast = Some(ToastMessage::new(toast_msg));
            }
        }
    }
}

pub fn render(_app: &App, frame: &mut Frame, area: Rect) {
    ensure_loaded();
    TEST_RESULT.get_or_init(|| Mutex::new(None));
    REFRESH_RESULT.get_or_init(|| Mutex::new(None));

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
        ProviderMode::DetailView(_) => {
            render_detail_view(frame, area, state);
        }
        _ => {
            render_list(frame, area, state);

            match state.mode {
                ProviderMode::List => {}
                ProviderMode::AddForm | ProviderMode::EditForm(_) => {
                    render_form_overlay(frame, area, state);
                }
                ProviderMode::DeleteConfirm(_) => {
                    render_delete_overlay(frame, area, state);
                }
                ProviderMode::DetailView(_) => unreachable!(),
            }
        }
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) {
    check_pending_results(app);

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
        ProviderMode::List => handle_list_key(app, key, state),
        ProviderMode::AddForm | ProviderMode::EditForm(_) => handle_form_key(app, key, state),
        ProviderMode::DeleteConfirm(_) => handle_delete_key(app, key, state),
        ProviderMode::DetailView(_) => handle_detail_key(app, key, state),
    }
}

fn render_loading(frame: &mut Frame, area: Rect) {
    let text = vec![
        Line::from(Span::styled(
            "Providers",
            Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Loading...",
            Style::default().fg(Color::Yellow),
        )),
    ];
    frame.render_widget(Paragraph::new(text), area);
}

fn render_list(frame: &mut Frame, area: Rect, state: &mut ProviderListState) {
    let header_cells = ["Name", "Protocol", "Base URL", "Status", "Key", "Models"];
    let header = Row::new(
        header_cells
            .iter()
            .map(|h| Cell::from(*h).style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD))),
    )
    .bottom_margin(1);

    let rows: Vec<Row> = state
        .providers
        .iter()
        .map(|p| {
            let status = if p.enabled {
                Span::styled("enabled", Style::default().fg(Color::Green))
            } else {
                Span::styled("disabled", Style::default().fg(Color::DarkGray))
            };
            let key_status = if p.has_credential {
                Span::styled("stored", Style::default().fg(Color::Green))
            } else {
                Span::styled("no key", Style::default().fg(Color::Red))
            };
            Row::new(vec![
                Cell::from(p.name.as_str()),
                Cell::from(p.protocol.as_str()).style(Style::default().fg(Color::DarkGray)),
                Cell::from(truncate_url(&p.base_url, 30)).style(Style::default().fg(Color::DarkGray)),
                Cell::from(status),
                Cell::from(key_status),
                Cell::from(p.model_count.to_string()).style(Style::default().fg(Color::DarkGray)),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(18),
            Constraint::Length(10),
            Constraint::Length(30),
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Length(7),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Span::styled(
                " Providers ",
                Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan),
            )),
    )
    .row_highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    );

    frame.render_stateful_widget(table, area, &mut state.table_state);

    let status_line = if TEST_IN_PROGRESS.load(Ordering::Relaxed) {
        Some(Span::styled(
            " Testing connection...",
            Style::default().fg(Color::Yellow),
        ))
    } else if REFRESH_IN_PROGRESS.load(Ordering::Relaxed) {
        Some(Span::styled(
            " Refreshing models...",
            Style::default().fg(Color::Yellow),
        ))
    } else {
        None
    };

    if let Some(status) = status_line {
        let status_area = Rect::new(
            area.x + 1,
            area.y + area.height.saturating_sub(2),
            area.width.saturating_sub(2),
            1,
        );
        frame.render_widget(Paragraph::new(Line::from(status)), status_area);
    }

    let hints = if state.providers.is_empty() {
        " a:Add  ?:Help "
    } else {
        " j/k:Nav  a:Add  e:Edit  d:Del  t:Toggle  Enter:Detail  T:Test  R:Refresh  ?:Help "
    };
    let hint = Paragraph::new(Span::styled(
        hints,
        Style::default().fg(Color::DarkGray),
    ));
    let hint_area = Rect::new(area.x, area.y + area.height.saturating_sub(1), area.width, 1);
    frame.render_widget(hint, hint_area);
}

fn render_detail_view(frame: &mut Frame, area: Rect, state: &mut ProviderListState) {
    let provider = match &state.detail_provider {
        Some(p) => p.clone(),
        None => {
            let idx = match state.mode {
                ProviderMode::DetailView(i) => i,
                _ => return,
            };
            if let Some(row) = state.providers.get(idx) {
                match load_full_provider(&row.id) {
                    Some(p) => {
                        state.detail_provider = Some(p.clone());
                        p
                    }
                    None => {
                        let msg = Paragraph::new("Provider not found. Press Esc to go back.")
                            .style(Style::default().fg(Color::Red));
                        frame.render_widget(msg, area);
                        return;
                    }
                }
            } else {
                let msg = Paragraph::new("Invalid selection. Press Esc to go back.")
                    .style(Style::default().fg(Color::Red));
                frame.render_widget(msg, area);
                return;
            }
        }
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    let status_text = if provider.enabled { "enabled" } else { "disabled" };
    let status_color = if provider.enabled { Color::Green } else { Color::DarkGray };
    let header_lines = vec![
        Line::from(vec![
            Span::styled(
                format!(" {} ", provider.name),
                Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan),
            ),
            Span::styled(
                format!(" {}", status_text),
                Style::default().fg(status_color).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(" URL: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&provider.base_url, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled(" Protocol: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&provider.protocol, Style::default().fg(Color::White)),
        ]),
        Line::from(""),
    ];

    let header_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    frame.render_widget(
        Paragraph::new(header_lines).block(header_block),
        chunks[0],
    );

    let override_count = provider.model_overrides.as_ref().map_or(0, |v| v.len());
    let total_rows = provider.models.len() + if override_count > 0 { override_count + 1 } else { 0 };
    let visible_height = chunks[1].height.saturating_sub(2) as usize;
    let max_scroll = total_rows.saturating_sub(visible_height);
    state.detail_scroll = state.detail_scroll.min(max_scroll);

    let model_header = Row::new(
        ["Model ID", "Context", "Max Out", "In $/M", "Out $/M"]
            .iter()
            .map(|h| Cell::from(*h).style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD))),
    )
    .bottom_margin(1);

    let mut all_rows: Vec<Row> = Vec::new();

    for m in &provider.models {
        all_rows.push(model_row(m, false));
    }

    if let Some(overrides) = &provider.model_overrides {
        if !overrides.is_empty() {
            all_rows.push(
                Row::new(vec![
                    Cell::from(Span::styled(
                        "── Overrides ──",
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                    )),
                    Cell::from(""),
                    Cell::from(""),
                    Cell::from(""),
                    Cell::from(""),
                ])
                .bottom_margin(1),
            );
            for m in overrides {
                all_rows.push(model_row(m, true));
            }
        }
    }

    let visible_rows: Vec<Row> = all_rows
        .into_iter()
        .skip(state.detail_scroll)
        .take(visible_height + 1)
        .collect();

    let title = format!(
        " Models ({}) {}",
        provider.models.len(),
        if override_count > 0 {
            format!("+ {} overrides", override_count)
        } else {
            String::new()
        }
    );

    let table = Table::new(
        visible_rows,
        [
            Constraint::Min(24),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
        ],
    )
    .header(model_header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Span::styled(
                format!(" {} ", title),
                Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan),
            )),
    );

    frame.render_widget(table, chunks[1]);

    let scroll_indicator = if total_rows > visible_height {
        let pos = state.detail_scroll;
        format!(" [{}/{}] ", pos + 1, total_rows.saturating_sub(visible_height) + 1)
    } else {
        String::new()
    };
    let hint_text = format!(" Esc:Back  j/k:Scroll{} ", scroll_indicator);
    let hint = Paragraph::new(Span::styled(
        hint_text,
        Style::default().fg(Color::DarkGray),
    ));
    frame.render_widget(hint, chunks[2]);
}

fn model_row(m: &ProviderModel, is_override: bool) -> Row<'_> {
    let id_display = if is_override {
        format!("[override] {}", m.id)
    } else {
        m.id.clone()
    };
    let id_style = if is_override {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::White)
    };
    Row::new(vec![
        Cell::from(id_display).style(id_style),
        Cell::from(format_tokens(m.context_window)).style(Style::default().fg(Color::DarkGray)),
        Cell::from(format_tokens(m.max_output_tokens)).style(Style::default().fg(Color::DarkGray)),
        Cell::from(format_cost_per_m(m.input_cost_per_1m)).style(Style::default().fg(Color::DarkGray)),
        Cell::from(format_cost_per_m(m.output_cost_per_1m)).style(Style::default().fg(Color::DarkGray)),
    ])
}

fn render_form_overlay(frame: &mut Frame, area: Rect, state: &mut ProviderListState) {
    let is_add = matches!(state.mode, ProviderMode::AddForm);
    let presets = presets::provider_presets();

    let form_width = 64.min(area.width);
    let form_height = if is_add {
        if state.show_overrides {
            34.min(area.height)
        } else {
            23.min(area.height)
        }
    } else if state.show_overrides {
        28.min(area.height)
    } else {
        17.min(area.height)
    };
    let x = (area.width.saturating_sub(form_width)) / 2;
    let y = (area.height.saturating_sub(form_height)) / 2;
    let popup_area = Rect::new(x, y, form_width, form_height);

    frame.render_widget(Clear, popup_area);

    let title = match state.mode {
        ProviderMode::EditForm(_) => " Edit Provider ",
        _ => " Add Provider ",
    };

    let block = Block::default()
        .title(title)
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    if is_add {
        let mut constraints: Vec<Constraint> = vec![
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(1),
        ];
        if state.show_overrides {
            constraints.push(Constraint::Min(0));
            constraints.push(Constraint::Length(1));
        } else {
            constraints.push(Constraint::Min(1));
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        let mut ci = 0;
        render_presets_row(frame, chunks[ci], state, &presets);
        ci += 1;

        let hint_desc = if state.preset_active && state.preset_index > 0 {
            if let Some(p) = presets.get(state.preset_index - 1) {
                format!(" ←/→:Navigate  Enter:Apply │ {} ", p.description)
            } else {
                " ←/→:Navigate  Enter:Apply ".to_string()
            }
        } else if state.preset_active {
            " ←/→:Navigate  Enter:Apply │ Start with empty fields ".to_string()
        } else {
            " ←/→:Presets  Tab:Next  Enter:Save  Esc:Cancel ".to_string()
        };
        let desc_p = Paragraph::new(Span::styled(
            hint_desc,
            Style::default().fg(Color::DarkGray),
        ));
        frame.render_widget(desc_p, chunks[ci]);
        ci += 1;

        update_textarea_block_styles(&mut state.form, state.preset_active, state.override_focused);

        frame.render_widget(&state.form.name, chunks[ci]);
        ci += 1;
        frame.render_widget(&state.form.base_url, chunks[ci]);
        ci += 1;
        frame.render_widget(&state.form.protocol, chunks[ci]);
        ci += 1;
        frame.render_widget(&state.form.api_key, chunks[ci]);
        ci += 1;

        render_overrides_header(frame, chunks[ci], state);
        ci += 1;

        if state.show_overrides {
            render_overrides_content(frame, chunks[ci], state);
            ci += 1;
        }

        let hint = Paragraph::new(Span::styled(
            " Tab:Next  Enter:Save  Esc:Cancel ",
            Style::default().fg(Color::DarkGray),
        ))
        .alignment(Alignment::Center);
        frame.render_widget(hint, chunks[ci]);
    } else {
        let mut constraints: Vec<Constraint> = vec![
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(1),
        ];
        if state.show_overrides {
            constraints.push(Constraint::Min(0));
            constraints.push(Constraint::Length(1));
        } else {
            constraints.push(Constraint::Min(1));
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        let mut ci = 0;

        update_textarea_block_styles(&mut state.form, false, state.override_focused);

        frame.render_widget(&state.form.name, chunks[ci]);
        ci += 1;
        frame.render_widget(&state.form.base_url, chunks[ci]);
        ci += 1;
        frame.render_widget(&state.form.protocol, chunks[ci]);
        ci += 1;
        frame.render_widget(&state.form.api_key, chunks[ci]);
        ci += 1;

        render_overrides_header(frame, chunks[ci], state);
        ci += 1;

        if state.show_overrides {
            render_overrides_content(frame, chunks[ci], state);
            ci += 1;
        }

        let hint = " Tab:Next  Enter:Save  Esc:Cancel  (leave key empty to keep)";
        let hint_p = Paragraph::new(Span::styled(hint, Style::default().fg(Color::DarkGray)))
            .alignment(Alignment::Center);
        frame.render_widget(hint_p, chunks[ci]);
    }
}

fn render_presets_row(
    frame: &mut Frame,
    area: Rect,
    state: &ProviderListState,
    presets: &[presets::ProviderPreset],
) {
    let total = presets.len() + 1;
    let selected = state.preset_index;
    let focused = state.preset_active;

    let mut spans: Vec<Span> = Vec::new();

    let blank_style = if selected == 0 && focused {
        Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else if selected == 0 {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    spans.push(Span::styled(" Blank ", blank_style));

    for (i, p) in presets.iter().enumerate() {
        let idx = i + 1;
        let style = if idx == selected && focused {
            Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else if idx == selected {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        spans.push(Span::raw(" "));
        let display_name = if p.name.len() > 22 {
            format!("{}…", &p.name[..20])
        } else {
            p.name.to_string()
        };
        spans.push(Span::styled(format!(" {} ", display_name), style));
    }

    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(" Presets ")
        .borders(Borders::ALL)
        .border_style(border_style);

    frame.render_widget(
        Paragraph::new(Line::from(spans)).block(block),
        area,
    );

    let indicator_text = if selected > 0 && selected <= presets.len() {
        format!(" {}/{} ", selected, total - 1)
    } else {
        format!(" {}/{} ", 0, total - 1)
    };
    let indicator_span = Span::styled(
        indicator_text.clone(),
        Style::default().fg(Color::DarkGray),
    );
    let indicator_len = indicator_text.len() as u16 + 2;
    let indicator_area = Rect::new(
        area.x + area.width.saturating_sub(indicator_len),
        area.y,
        indicator_len,
        1,
    );
    frame.render_widget(Paragraph::new(Line::from(indicator_span)), indicator_area);
}

fn render_overrides_header(frame: &mut Frame, area: Rect, state: &ProviderListState) {
    let count = state.model_overrides.len();
    let focused = state.override_focused;
    let header_text = if state.show_overrides {
        format!(" Model Overrides: {} items  Enter:collapse ", count)
    } else {
        format!(" Model Overrides: {} items  Enter:expand ", count)
    };
    let style = if focused {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    frame.render_widget(Paragraph::new(Span::styled(header_text, style)), area);
}

fn render_overrides_content(frame: &mut Frame, area: Rect, state: &mut ProviderListState) {
    let mut lines: Vec<Line> = Vec::new();
    let inner_w = area.width.saturating_sub(2) as usize;

    lines.push(Line::from(Span::styled(
        format!("┌{}┐", "─".repeat(inner_w)),
        Style::default().fg(Color::DarkGray),
    )));

    let available = area.height.saturating_sub(3) as usize;
    let visible = state.model_overrides.len().min(available);

    for (i, mo) in state.model_overrides.iter().enumerate().take(visible) {
        let is_selected = i == state.override_selected && !state.adding_override;
        let style = if is_selected {
            Style::default().fg(Color::Black).bg(Color::DarkGray)
        } else {
            Style::default().fg(Color::Yellow)
        };
        let id_str = if mo.id.len() > 18 {
            format!("{}…", &mo.id[..17])
        } else {
            mo.id.clone()
        };
        let ctx = format_tokens(mo.context_window);
        let max_out = format_tokens(mo.max_output_tokens);
        let inp = format_cost_per_m(mo.input_cost_per_1m);
        let out = format_cost_per_m(mo.output_cost_per_1m);
        let proto = mo.protocol.as_deref().unwrap_or("-");
        let row = format!(
            "│ {:<18} {:>6} {:>6} {:>7} {:>7} {:>6}│",
            id_str, ctx, max_out, inp, out, proto
        );
        lines.push(Line::from(Span::styled(row, style)));
    }

    lines.push(Line::from(Span::styled(
        format!("└{}┘", "─".repeat(inner_w)),
        Style::default().fg(Color::DarkGray),
    )));

    if state.adding_override {
        let form = &state.override_form;
        let fields: [(&str, &str, usize); 6] = [
            ("id", &form.model_id, 0),
            ("ctx", &form.context_window, 1),
            ("max", &form.max_output, 2),
            ("$in", &form.input_cost, 3),
            ("$out", &form.output_cost, 4),
            ("proto", &form.protocol, 5),
        ];
        let mut spans: Vec<Span> = vec![Span::styled(
            "Add: ",
            Style::default().fg(Color::Cyan),
        )];
        for (label, val, idx) in &fields {
            if *idx == form.focused {
                spans.push(Span::styled(
                    format!("{}=[{}]", label, val),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::REVERSED),
                ));
            } else {
                spans.push(Span::styled(
                    format!("{}=[{}]", label, val),
                    Style::default().fg(Color::DarkGray),
                ));
            }
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(
            "Enter:Ok Esc:Cancel",
            Style::default().fg(Color::DarkGray),
        ));
        lines.push(Line::from(spans));
    } else if !state.model_overrides.is_empty() {
        lines.push(Line::from(Span::styled(
            " j/k:nav  d:del  a:add  Enter:collapse",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            " a:add override  Enter:collapse",
            Style::default().fg(Color::DarkGray),
        )));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

fn update_textarea_block_styles(form: &mut ProviderFormState, preset_active: bool, override_focused: bool) {
    let focused = form.focused;
    let pairs: [(&str, &mut TextArea<'static>); 4] = [
        ("Name", &mut form.name),
        ("Base URL", &mut form.base_url),
        ("Protocol", &mut form.protocol),
        ("API Key", &mut form.api_key),
    ];

    for (i, (label, ta)) in pairs.into_iter().enumerate() {
        let style = if preset_active || override_focused {
            Style::default().fg(Color::DarkGray)
        } else if i == focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        ta.set_block(
            Block::default()
                .title(Line::from(format!(" {} ", label)))
                .borders(Borders::ALL)
                .border_style(style),
        );
    }
}

fn render_delete_overlay(frame: &mut Frame, area: Rect, state: &ProviderListState) {
    let idx = match state.mode {
        ProviderMode::DeleteConfirm(i) => i,
        _ => return,
    };
    let name = state.providers.get(idx).map(|p| p.name.as_str()).unwrap_or("?");
    let msg = format!("Delete provider '{}'?", name);

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
        Paragraph::new(Span::styled(
            msg,
            Style::default().fg(Color::White),
        ))
        .alignment(Alignment::Center),
        Paragraph::new(""),
        Paragraph::new(Span::styled(
            " y:Confirm  Esc:Cancel ",
            Style::default().fg(Color::DarkGray),
        ))
        .alignment(Alignment::Center),
    ];

    let text_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);

    for (i, p) in lines.into_iter().enumerate() {
        if i < text_chunks.len() {
            frame.render_widget(p, text_chunks[i]);
        }
    }
}

pub fn is_form_active() -> bool {
    if let Some(state_ref) = STATE.get() {
        if let Ok(guard) = state_ref.lock() {
            if let Some(state) = guard.as_ref() {
                return matches!(
                    state.mode,
                    ProviderMode::AddForm
                        | ProviderMode::EditForm(_)
                        | ProviderMode::DeleteConfirm(_)
                );
            }
        }
    }
    false
}

fn handle_list_key(app: &mut App, key: KeyEvent, state: &mut ProviderListState) {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if !state.providers.is_empty() {
                let i = state.table_state.selected().unwrap_or(0);
                let next = (i + 1).min(state.providers.len() - 1);
                state.table_state.select(Some(next));
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if !state.providers.is_empty() {
                let i = state.table_state.selected().unwrap_or(0);
                let prev = i.saturating_sub(1);
                state.table_state.select(Some(prev));
            }
        }
        KeyCode::Enter => {
            if let Some(i) = state.table_state.selected() {
                if i < state.providers.len() {
                    state.mode = ProviderMode::DetailView(i);
                    state.detail_provider = None;
                    state.detail_scroll = 0;
                }
            }
        }
        KeyCode::Char('a') => {
            state.form = ProviderFormState::new_empty();
            state.mode = ProviderMode::AddForm;
            state.preset_active = false;
            state.preset_index = 0;
            state.model_overrides = Vec::new();
            state.show_overrides = false;
            state.override_selected = 0;
            state.override_focused = false;
            state.adding_override = false;
            state.override_form = OverrideFormState::new();
        }
        KeyCode::Char('e') => {
            if let Some(i) = state.table_state.selected() {
                if i < state.providers.len() {
                    let p = &state.providers[i];
                    state.form = ProviderFormState::with_values(
                        &p.name,
                        &p.base_url,
                        &p.protocol,
                        "",
                    );
                    let overrides = load_full_provider(&p.id)
                        .and_then(|p| p.model_overrides)
                        .unwrap_or_default();
                    state.model_overrides = overrides;
                    state.show_overrides = !state.model_overrides.is_empty();
                    state.override_selected = 0;
                    state.override_focused = false;
                    state.adding_override = false;
                    state.override_form = OverrideFormState::new();
                    state.mode = ProviderMode::EditForm(i);
                }
            }
        }
        KeyCode::Char('d') => {
            if let Some(i) = state.table_state.selected() {
                if i < state.providers.len() {
                    state.mode = ProviderMode::DeleteConfirm(i);
                }
            }
        }
        KeyCode::Char('t') => {
            if let Some(i) = state.table_state.selected() {
                if i < state.providers.len() {
                    do_toggle_provider(app, i, &state.providers);
                    state.reload();
                }
            }
        }
        KeyCode::Char('T') => {
            if let Some(i) = state.table_state.selected() {
                if i < state.providers.len() && !TEST_IN_PROGRESS.load(Ordering::Relaxed) {
                    let p = &state.providers[i];
                    do_test_connection(&p.base_url, &p.name);
                }
            }
        }
        KeyCode::Char('R') => {
            if let Some(i) = state.table_state.selected() {
                if i < state.providers.len() && !REFRESH_IN_PROGRESS.load(Ordering::Relaxed) {
                    let p = &state.providers[i];
                    do_refresh_models(&p.id, &p.name);
                }
            }
        }
        _ => {}
    }
}

fn handle_form_key(app: &mut App, key: KeyEvent, state: &mut ProviderListState) {
    let is_add = matches!(state.mode, ProviderMode::AddForm);

    if is_add && state.preset_active {
        match key.code {
            KeyCode::Left => {
                let presets = presets::provider_presets();
                let max = presets.len();
                state.preset_index = state.preset_index.saturating_sub(1);
                if state.preset_index > max {
                    state.preset_index = max;
                }
                return;
            }
            KeyCode::Right => {
                let presets = presets::provider_presets();
                let max = presets.len();
                state.preset_index = (state.preset_index + 1).min(max);
                return;
            }
            KeyCode::Enter => {
                apply_preset(state);
                state.preset_active = false;
                return;
            }
            KeyCode::Esc => {
                state.preset_active = false;
                return;
            }
            KeyCode::Tab => {
                state.preset_active = false;
                state.form.focused = 0;
                return;
            }
            KeyCode::BackTab => {
                state.preset_active = false;
                state.form.focused = 3;
                return;
            }
            _ => return,
        }
    }

    if state.override_focused {
        match key.code {
            KeyCode::Esc => {
                if state.adding_override {
                    state.adding_override = false;
                    state.override_form.clear();
                } else {
                    state.override_focused = false;
                }
                return;
            }
            KeyCode::Tab => {
                if state.adding_override {
                    state.override_form.next_field();
                } else {
                    state.override_focused = false;
                    state.form.focused = 0;
                }
                return;
            }
            KeyCode::BackTab => {
                if state.adding_override {
                    state.override_form.prev_field();
                } else {
                    state.override_focused = false;
                    state.form.focused = 3;
                }
                return;
            }
            KeyCode::Enter => {
                if state.adding_override {
                    if let Some(mo) = state.override_form.to_provider_model() {
                        state.model_overrides.push(mo);
                        state.override_form.clear();
                        state.override_selected =
                            state.model_overrides.len().saturating_sub(1);
                    }
                    state.adding_override = false;
                } else if state.show_overrides {
                    state.show_overrides = false;
                } else {
                    state.show_overrides = true;
                }
                return;
            }
            KeyCode::Char('a') if !state.adding_override => {
                state.adding_override = true;
                state.override_form = OverrideFormState::new();
                return;
            }
            KeyCode::Char('d') if !state.adding_override => {
                if !state.model_overrides.is_empty() {
                    let idx = state
                        .override_selected
                        .min(state.model_overrides.len() - 1);
                    state.model_overrides.remove(idx);
                    if state.override_selected >= state.model_overrides.len() {
                        state.override_selected =
                            state.model_overrides.len().saturating_sub(1);
                    }
                }
                return;
            }
            KeyCode::Char('j') | KeyCode::Down if !state.adding_override => {
                if !state.model_overrides.is_empty() {
                    state.override_selected = (state.override_selected + 1)
                        .min(state.model_overrides.len() - 1);
                }
                return;
            }
            KeyCode::Char('k') | KeyCode::Up if !state.adding_override => {
                state.override_selected = state.override_selected.saturating_sub(1);
                return;
            }
            KeyCode::Backspace if state.adding_override => {
                state.override_form.active_field().pop();
                return;
            }
            KeyCode::Char(c) if state.adding_override => {
                state.override_form.active_field().push(c);
                return;
            }
            _ => return,
        }
    }

    match key.code {
        KeyCode::Tab => {
            if state.form.focused >= 3 {
                state.override_focused = true;
            } else {
                state.form.focused += 1;
            }
        }
        KeyCode::BackTab => {
            if state.form.focused == 0 {
                state.override_focused = true;
            } else {
                state.form.focused -= 1;
            }
        }
        KeyCode::Left if is_add => {
            state.preset_active = true;
        }
        KeyCode::Right if is_add => {
            state.preset_active = true;
        }
        KeyCode::Enter => {
            submit_form(app, state);
        }
        KeyCode::Esc => {
            state.mode = ProviderMode::List;
        }
        _ => {
            state.form.active_textarea().input(key);
        }
    }
}

fn handle_delete_key(app: &mut App, key: KeyEvent, state: &mut ProviderListState) {
    match key.code {
        KeyCode::Char('y') => {
            let idx = match state.mode {
                ProviderMode::DeleteConfirm(i) => i,
                _ => return,
            };
            if idx < state.providers.len() {
                let name = state.providers[idx].name.clone();
                match do_delete_provider(idx, &state.providers) {
                    Ok(()) => {
                        app.toast = Some(ToastMessage::new(format!("Deleted '{}'", name)));
                        state.reload();
                    }
                    Err(e) => {
                        app.toast = Some(ToastMessage::new(format!("Error: {}", e)));
                    }
                }
            }
            state.mode = ProviderMode::List;
        }
        KeyCode::Esc => {
            state.mode = ProviderMode::List;
        }
        _ => {}
    }
}

fn handle_detail_key(_app: &mut App, key: KeyEvent, state: &mut ProviderListState) {
    let provider = match &state.detail_provider {
        Some(p) => p.clone(),
        None => return,
    };

    match key.code {
        KeyCode::Esc => {
            state.mode = ProviderMode::List;
            state.detail_provider = None;
            state.detail_scroll = 0;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            let override_count = provider.model_overrides.as_ref().map_or(0, |v| v.len());
            let total = provider.models.len() + if override_count > 0 { override_count + 1 } else { 0 };
            let max_scroll = total.saturating_sub(1);
            state.detail_scroll = (state.detail_scroll + 1).min(max_scroll);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.detail_scroll = state.detail_scroll.saturating_sub(1);
        }
        _ => {}
    }
}

fn apply_preset(state: &mut ProviderListState) {
    let presets = presets::provider_presets();
    let idx = state.preset_index;

    if idx == 0 {
        state.form = ProviderFormState::new_empty();
        state.model_overrides = Vec::new();
        state.show_overrides = false;
        state.override_selected = 0;
        return;
    }

    if let Some(preset) = presets.get(idx - 1) {
        state.form = ProviderFormState::with_values(
            preset.name,
            preset.base_url,
            preset.protocol,
            "",
        );
        state.form.focused = 0;
        state.model_overrides = preset.model_overrides.clone();
        state.show_overrides = !preset.model_overrides.is_empty();
        state.override_selected = 0;
    }
}

fn submit_form(app: &mut App, state: &mut ProviderListState) {
    let form = &state.form;
    let result = form.validate();
    match result {
        Ok((name, base_url, protocol, api_key)) => {
            let model_overrides = if state.model_overrides.is_empty() {
                None
            } else {
                Some(state.model_overrides.clone())
            };

            let outcome = match state.mode {
                ProviderMode::AddForm => {
                    do_add_provider(&name, &base_url, &protocol, &api_key, model_overrides)
                }
                ProviderMode::EditForm(idx) => {
                    let has_key = !api_key.is_empty();
                    let rows = &state.providers;
                    if idx < rows.len() {
                        do_edit_provider(
                            idx,
                            &name,
                            &base_url,
                            &protocol,
                            if has_key { Some(&api_key) } else { None },
                            rows,
                            model_overrides,
                        )
                    } else {
                        Err("Invalid selection".into())
                    }
                }
                _ => Ok(()),
            };

            match outcome {
                Ok(()) => {
                    app.toast = Some(ToastMessage::new("Provider saved"));
                    state.reload();
                    state.mode = ProviderMode::List;
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

fn do_add_provider(
    name: &str,
    base_url: &str,
    protocol: &str,
    api_key: &str,
    model_overrides: Option<Vec<ProviderModel>>,
) -> Result<(), String> {
    let id = slugify(name);
    let credential_key = id.clone();

    let duplicate = store::load_providers()
        .unwrap_or_default()
        .iter()
        .any(|p| p.id == id);
    if duplicate {
        return Err(format!("Provider '{}' already exists", id));
    }

    store::update_providers_with_lock(|providers| {
        providers.push(Provider {
            id: id.clone(),
            name: name.to_string(),
            protocol: protocol.to_string(),
            base_url: base_url.to_string(),
            credential_key: credential_key.clone(),
            daily_token_quota: None,
            daily_request_quota: None,
            quota_reset_utc_hour: 0,
            enabled: true,
            models: vec![],
            model_overrides,
        });
    })
    .map_err(|e| e.to_string())?;

    blocking_store_credential(&credential_key, api_key).map_err(|e| e.to_string())?;
    notify_sidecar_reload();

    Ok(())
}

fn do_edit_provider(
    idx: usize,
    name: &str,
    base_url: &str,
    protocol: &str,
    new_api_key: Option<&str>,
    rows: &[ProviderRow],
    model_overrides: Option<Vec<ProviderModel>>,
) -> Result<(), String> {
    let row = rows.get(idx).ok_or("Invalid selection")?;
    let id = row.id.clone();

    store::update_providers_with_lock(|providers| {
        if let Some(p) = providers.iter_mut().find(|p| p.id == id) {
            p.name = name.to_string();
            p.protocol = protocol.to_string();
            p.base_url = base_url.to_string();
            p.model_overrides = model_overrides;
        }
    })
    .map_err(|e| e.to_string())?;

    if let Some(key) = new_api_key {
        blocking_store_credential(&id, key).map_err(|e| e.to_string())?;
    }
    notify_sidecar_reload();

    Ok(())
}

fn do_delete_provider(idx: usize, rows: &[ProviderRow]) -> Result<(), String> {
    let row = rows.get(idx).ok_or("Invalid selection")?;
    let id = row.id.clone();

    store::update_providers_with_lock(|providers| {
        providers.retain(|p| p.id != id);
    })
    .map_err(|e| e.to_string())?;

    let _ = blocking_delete_credential(&id);
    notify_sidecar_reload();

    Ok(())
}

fn do_toggle_provider(app: &mut App, idx: usize, rows: &[ProviderRow]) {
    let row = match rows.get(idx) {
        Some(r) => r,
        None => {
            app.toast = Some(ToastMessage::new("No provider selected"));
            return;
        }
    };
    let id = row.id.clone();
    let new_state = !row.enabled;

    match store::update_providers_with_lock(|providers| {
        if let Some(p) = providers.iter_mut().find(|p| p.id == id) {
            p.enabled = new_state;
        }
    }) {
        Ok(()) => {
            let label = if new_state { "enabled" } else { "disabled" };
            app.toast = Some(ToastMessage::new(format!("{} '{}'", label, row.name)));
        }
        Err(e) => {
            app.toast = Some(ToastMessage::new(format!("Error: {}", e)));
        }
    }
}

fn do_test_connection(base_url: &str, provider_name: &str) {
    TEST_IN_PROGRESS.store(true, Ordering::Relaxed);
    let url = base_url.to_string();
    let pname = provider_name.to_string();
    let result_lock = TEST_RESULT.get_or_init(|| Mutex::new(None));

    let lock_ref = result_lock;
    std::thread::Builder::new()
        .name("provider-test".into())
        .spawn(move || {
            let client = reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_default();

            let base = url.trim_end_matches('/');
            let health_url = format!("{}/health", base);
            let models_url = format!("{}/v1/models", base);

            let result: (String, bool) = match client.get(&health_url).send() {
                Ok(resp) if resp.status().is_success() => {
                    (format!("Connection to '{}' successful", pname), true)
                }
                _ => {
                    match client.head(&models_url).send() {
                        Ok(resp) if resp.status().is_success() => {
                            (format!("Connection to '{}' successful", pname), true)
                        }
                        Ok(resp) => {
                            (format!("Connection to '{}' returned HTTP {}", pname, resp.status()), false)
                        }
                        Err(e) => {
                            (format!("Connection to '{}' failed: {}", pname, e), false)
                        }
                    }
                }
            };

            TEST_IN_PROGRESS.store(false, Ordering::Relaxed);
            if let Ok(mut g) = lock_ref.lock() {
                *g = Some(result);
            }
        })
        .ok();
}

fn do_refresh_models(provider_id: &str, provider_name: &str) {
    REFRESH_IN_PROGRESS.store(true, Ordering::Relaxed);
    let pid = provider_id.to_string();
    let pname = provider_name.to_string();
    let result_lock = REFRESH_RESULT.get_or_init(|| Mutex::new(None));

    let lock_ref = result_lock;
    std::thread::Builder::new()
        .name("provider-refresh".into())
        .spawn(move || {
            let result: (String, bool) = match tokio_runtime().block_on(async {
                let client = reqwest::Client::new();
                coderouter_proxy::models::refresher::refresh_provider_models(pid, &client).await
            }) {
                Ok(models) => {
                    notify_sidecar_reload();
                    (format!("Refreshed {} models for '{}'", models.len(), pname), true)
                }
                Err(e) => {
                    (format!("Refresh failed for '{}': {}", pname, e), false)
                }
            };

            REFRESH_IN_PROGRESS.store(false, Ordering::Relaxed);
            if let Ok(mut g) = lock_ref.lock() {
                *g = Some(result);
            }
        })
        .ok();
}
