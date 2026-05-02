use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use crate::app::{App, ToastMessage};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Wrap};
use tui_textarea::TextArea;

use coderouter_proxy::config::store;
use coderouter_proxy::opencode::config_writer;
use coderouter_proxy::opencode::custom_agents;

#[derive(Clone)]
struct AgentMappingRow {
    name: String,
    group: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum FocusSection {
    MappingTable,
    CustomAgents,
}

#[derive(Clone, PartialEq)]
enum OpenCodeMode {
    List,
    EditMapping(usize),
    SetCustomPath,
    TemplateSelect,
    AgentForm(Option<String>),
    AgentDeleteConfirm(usize),
    AgentDetail(usize),
}

const AGENT_NAMES: &[&str] = &[
    "build",
    "plan",
    "general",
    "explore",
    "compaction",
    "title",
    "summary",
    "small_model",
];

const PERM_OPTIONS: &[&str] = &["none", "allow", "deny", "ask"];
const MODE_OPTIONS: &[&str] = &["subagent", "all", "primary"];
const FORM_FIELD_COUNT: usize = 14;

struct AgentFormState {
    original_name: Option<String>,
    name: TextArea<'static>,
    description: TextArea<'static>,
    prompt: TextArea<'static>,
    mode_idx: usize,
    group_idx: usize,
    steps_buf: String,
    top_p_buf: String,
    color_buf: String,
    hidden: bool,
    disabled: bool,
    perm_edit: usize,
    perm_bash: usize,
    perm_webfetch: usize,
    perm_task: usize,
    focused: usize,
}

struct OpenCodeState {
    config_path: Option<PathBuf>,
    config_exists: bool,
    provider_enabled: bool,
    agent_mappings: Vec<AgentMappingRow>,
    available_groups: Vec<String>,
    mode: OpenCodeMode,
    selected_row: usize,
    mapping_popup_idx: usize,
    custom_path_input: TextArea<'static>,
    focus_section: FocusSection,
    custom_agents: Vec<custom_agents::CustomAgent>,
    custom_agent_selected: usize,
    custom_agent_scroll: usize,
    template_selected: usize,
    agent_form: Option<AgentFormState>,
}

static STATE: OnceLock<Mutex<Option<OpenCodeState>>> = OnceLock::new();

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

fn detect_provider_enabled(config_path: &PathBuf) -> bool {
    let contents = match std::fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let config: serde_json::Value = match serde_json::from_str(&contents) {
        Ok(v) => v,
        Err(_) => return false,
    };
    config
        .get("provider")
        .and_then(|p| p.get("coderouter"))
        .is_some()
}

fn load_mappings(config_path: &PathBuf) -> Vec<AgentMappingRow> {
    let mapping = config_writer::get_current_agent_mapping(config_path).unwrap_or_default();
    AGENT_NAMES
        .iter()
        .map(|name| {
            let group = match *name {
                "build" => mapping.build.clone(),
                "plan" => mapping.plan.clone(),
                "general" => mapping.general.clone(),
                "explore" => mapping.explore.clone(),
                "compaction" => mapping.compaction.clone(),
                "title" => mapping.title.clone(),
                "summary" => mapping.summary.clone(),
                "small_model" => mapping.small_model.clone(),
                _ => None,
            };
            AgentMappingRow {
                name: name.to_string(),
                group,
            }
        })
        .collect()
}

fn load_custom_agents() -> Vec<custom_agents::CustomAgent> {
    custom_agents::list_agents().unwrap_or_default()
}

fn perm_to_idx(perm: Option<&custom_agents::PermissionLevel>) -> usize {
    match perm {
        Some(custom_agents::PermissionLevel::Allow) => 1,
        Some(custom_agents::PermissionLevel::Deny) => 2,
        Some(custom_agents::PermissionLevel::Ask) => 3,
        _ => 0,
    }
}

fn perm_bash_to_idx(perm: Option<&custom_agents::BashPermission>) -> usize {
    match perm {
        Some(custom_agents::BashPermission::Simple(p)) => perm_to_idx(Some(p)),
        _ => 0,
    }
}

fn perm_task_to_idx(perm: Option<&std::collections::HashMap<String, custom_agents::PermissionLevel>>) -> usize {
    match perm {
        Some(map) if !map.is_empty() => {
            if map.values().any(|v| matches!(v, custom_agents::PermissionLevel::Allow)) {
                1
            } else if map
                .values()
                .any(|v| matches!(v, custom_agents::PermissionLevel::Deny))
            {
                2
            } else {
                3
            }
        }
        _ => 0,
    }
}

fn idx_to_perm(idx: usize) -> Option<custom_agents::PermissionLevel> {
    match idx {
        1 => Some(custom_agents::PermissionLevel::Allow),
        2 => Some(custom_agents::PermissionLevel::Deny),
        3 => Some(custom_agents::PermissionLevel::Ask),
        _ => None,
    }
}

fn idx_to_mode(idx: usize) -> custom_agents::AgentMode {
    match idx {
        1 => custom_agents::AgentMode::All,
        2 => custom_agents::AgentMode::Primary,
        _ => custom_agents::AgentMode::Subagent,
    }
}

fn mode_to_idx(mode: &custom_agents::AgentMode) -> usize {
    match mode {
        custom_agents::AgentMode::Subagent => 0,
        custom_agents::AgentMode::All => 1,
        custom_agents::AgentMode::Primary => 2,
    }
}

impl AgentFormState {
    fn new_blank() -> Self {
        Self {
            original_name: None,
            name: make_textarea(""),
            description: make_textarea(""),
            prompt: make_textarea(""),
            mode_idx: 0,
            group_idx: 0,
            steps_buf: String::new(),
            top_p_buf: String::new(),
            color_buf: String::new(),
            hidden: false,
            disabled: false,
            perm_edit: 0,
            perm_bash: 0,
            perm_webfetch: 0,
            perm_task: 0,
            focused: 0,
        }
    }

    fn from_template(tmpl: &custom_agents::AgentTemplate) -> Self {
        Self {
            original_name: None,
            name: make_textarea(""),
            description: make_textarea(&tmpl.agent.description),
            prompt: make_textarea(&tmpl.agent.prompt),
            mode_idx: mode_to_idx(&tmpl.agent.mode),
            group_idx: 0,
            steps_buf: tmpl.agent.steps.map(|s| s.to_string()).unwrap_or_default(),
            top_p_buf: tmpl.agent.top_p.map(|p| p.to_string()).unwrap_or_default(),
            color_buf: tmpl.agent.color.clone().unwrap_or_default(),
            hidden: tmpl.agent.hidden.unwrap_or(false),
            disabled: tmpl.agent.disable.unwrap_or(false),
            perm_edit: perm_to_idx(tmpl.agent.permission.as_ref().and_then(|p| p.edit.as_ref())),
            perm_bash: perm_bash_to_idx(tmpl.agent.permission.as_ref().and_then(|p| p.bash.as_ref())),
            perm_webfetch: perm_to_idx(tmpl.agent.permission.as_ref().and_then(|p| p.webfetch.as_ref())),
            perm_task: perm_task_to_idx(tmpl.agent.permission.as_ref().and_then(|p| p.task.as_ref())),
            focused: 0,
        }
    }

    fn from_agent(agent: &custom_agents::CustomAgent, available_groups: &[String]) -> Self {
        let group_idx = agent
            .model
            .as_ref()
            .and_then(|m| available_groups.iter().position(|g| g == m))
            .map(|p| p + 1)
            .unwrap_or(0);
        Self {
            original_name: Some(agent.name.clone()),
            name: make_textarea(&agent.name),
            description: make_textarea(&agent.description),
            prompt: make_textarea(&agent.prompt),
            mode_idx: mode_to_idx(&agent.mode),
            group_idx,
            steps_buf: agent.steps.map(|s| s.to_string()).unwrap_or_default(),
            top_p_buf: agent.top_p.map(|p| p.to_string()).unwrap_or_default(),
            color_buf: agent.color.clone().unwrap_or_default(),
            hidden: agent.hidden.unwrap_or(false),
            disabled: agent.disable.unwrap_or(false),
            perm_edit: perm_to_idx(agent.permission.as_ref().and_then(|p| p.edit.as_ref())),
            perm_bash: perm_bash_to_idx(agent.permission.as_ref().and_then(|p| p.bash.as_ref())),
            perm_webfetch: perm_to_idx(agent.permission.as_ref().and_then(|p| p.webfetch.as_ref())),
            perm_task: perm_task_to_idx(agent.permission.as_ref().and_then(|p| p.task.as_ref())),
            focused: 0,
        }
    }

    fn to_agent(&self, available_groups: &[String]) -> custom_agents::CustomAgent {
        let model = if self.group_idx > 0 && self.group_idx <= available_groups.len() {
            Some(available_groups[self.group_idx - 1].clone())
        } else {
            None
        };
        let steps = self.steps_buf.parse::<u64>().ok();
        let top_p = self.top_p_buf.parse::<f64>().ok();

        let permission = {
            let edit = idx_to_perm(self.perm_edit);
            let bash = idx_to_perm(self.perm_bash).map(custom_agents::BashPermission::Simple);
            let webfetch = idx_to_perm(self.perm_webfetch);
            let task: Option<HashMap<String, custom_agents::PermissionLevel>> =
                if self.perm_task > 0 {
                    let perm = idx_to_perm(self.perm_task)
                        .unwrap_or(custom_agents::PermissionLevel::Allow);
                    Some(HashMap::from([("*".to_string(), perm)]))
                } else {
                    None
                };
            if edit.is_some() || bash.is_some() || webfetch.is_some() || task.is_some() {
                Some(custom_agents::AgentPermissions {
                    edit,
                    bash,
                    webfetch,
                    task,
                })
            } else {
                None
            }
        };

        custom_agents::CustomAgent {
            name: get_ta_val(&self.name),
            description: get_ta_val(&self.description),
            mode: idx_to_mode(self.mode_idx),
            model,
            prompt: get_ta_val(&self.prompt),
            temperature: None,
            steps,
            disable: if self.disabled { Some(true) } else { None },
            hidden: if self.hidden { Some(true) } else { None },
            color: if self.color_buf.is_empty() {
                None
            } else {
                Some(self.color_buf.clone())
            },
            top_p,
            permission,
            ..Default::default()
        }
    }
}

impl OpenCodeState {
    fn load() -> Self {
        let app_config = store::load_app_config().unwrap_or_default();
        let config_path =
            config_writer::resolve_opencode_config_path(app_config.opencode_config_path.as_deref());
        let config_exists = config_path
            .as_ref()
            .map(|p| p.exists())
            .unwrap_or(false);
        let provider_enabled = config_path
            .as_ref()
            .filter(|p| p.exists())
            .map(|p| detect_provider_enabled(p))
            .unwrap_or(false);
        let agent_mappings = config_path
            .as_ref()
            .filter(|p| p.exists())
            .map(|p| load_mappings(p))
            .unwrap_or_else(|| {
                AGENT_NAMES
                    .iter()
                    .map(|n| AgentMappingRow {
                        name: n.to_string(),
                        group: None,
                    })
                    .collect()
            });
        let available_groups = store::load_groups()
            .unwrap_or_default()
            .into_iter()
            .map(|g| g.alias)
            .collect();
        let custom_agents = load_custom_agents();
        Self {
            config_path,
            config_exists,
            provider_enabled,
            agent_mappings,
            available_groups,
            mode: OpenCodeMode::List,
            selected_row: 0,
            mapping_popup_idx: 0,
            custom_path_input: make_textarea(""),
            focus_section: FocusSection::MappingTable,
            custom_agents,
            custom_agent_selected: 0,
            custom_agent_scroll: 0,
            template_selected: 0,
            agent_form: None,
        }
    }

    fn reload(&mut self) {
        let app_config = store::load_app_config().unwrap_or_default();
        self.config_path =
            config_writer::resolve_opencode_config_path(app_config.opencode_config_path.as_deref());
        self.config_exists = self
            .config_path
            .as_ref()
            .map(|p| p.exists())
            .unwrap_or(false);
        self.provider_enabled = self
            .config_path
            .as_ref()
            .filter(|p| p.exists())
            .map(|p| detect_provider_enabled(p))
            .unwrap_or(false);
        self.agent_mappings = self
            .config_path
            .as_ref()
            .filter(|p| p.exists())
            .map(|p| load_mappings(p))
            .unwrap_or_else(|| {
                AGENT_NAMES
                    .iter()
                    .map(|n| AgentMappingRow {
                        name: n.to_string(),
                        group: None,
                    })
                    .collect()
            });
        self.available_groups = store::load_groups()
            .unwrap_or_default()
            .into_iter()
            .map(|g| g.alias)
            .collect();
        self.custom_agents = load_custom_agents();
        if self.selected_row >= self.agent_mappings.len() && !self.agent_mappings.is_empty() {
            self.selected_row = self.agent_mappings.len() - 1;
        }
        if self.custom_agent_selected >= self.custom_agents.len()
            && !self.custom_agents.is_empty()
        {
            self.custom_agent_selected = self.custom_agents.len() - 1;
        }
    }
}

fn ensure_loaded() {
    STATE.get_or_init(|| Mutex::new(Some(OpenCodeState::load())));
}

pub fn render(_app: &App, frame: &mut Frame, area: Rect) {
    ensure_loaded();

    let state_ref = match STATE.get() {
        Some(s) => s,
        None => {
            render_loading(frame, area);
            return;
        }
    };
    let mut guard = match state_ref.lock() {
        Ok(g) => g,
        Err(_) => {
            render_loading(frame, area);
            return;
        }
    };
    let state = match guard.as_mut() {
        Some(s) => s,
        None => {
            render_loading(frame, area);
            return;
        }
    };

    match state.mode {
        OpenCodeMode::AgentDetail(_) => {
            render_agent_detail(frame, area, state);
        }
        _ => {
            render_main(frame, area, state);
            match state.mode {
                OpenCodeMode::EditMapping(_) => render_mapping_popup(frame, area, state),
                OpenCodeMode::SetCustomPath => render_custom_path_popup(frame, area, state),
                OpenCodeMode::TemplateSelect => render_template_select_popup(frame, area, state),
                OpenCodeMode::AgentForm(_) => render_agent_form_popup(frame, area, state),
                OpenCodeMode::AgentDeleteConfirm(_) => {
                    render_agent_delete_popup(frame, area, state)
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
        OpenCodeMode::List => handle_list_key(app, key, state),
        OpenCodeMode::EditMapping(_) => handle_mapping_key(app, key, state),
        OpenCodeMode::SetCustomPath => handle_custom_path_key(app, key, state),
        OpenCodeMode::TemplateSelect => handle_template_select_key(app, key, state),
        OpenCodeMode::AgentForm(_) => handle_agent_form_key(app, key, state),
        OpenCodeMode::AgentDeleteConfirm(_) => handle_agent_delete_key(app, key, state),
        OpenCodeMode::AgentDetail(_) => handle_agent_detail_key(app, key, state),
    }
}

fn render_loading(frame: &mut Frame, area: Rect) {
    let text = vec![
        Line::from(Span::styled(
            "OpenCode Config",
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

fn render_main(frame: &mut Frame, area: Rect, state: &mut OpenCodeState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(4),
            Constraint::Min(6),
            Constraint::Min(4),
            Constraint::Length(1),
        ])
        .split(area);

    render_config_path_section(frame, chunks[0], state);
    render_provider_section(frame, chunks[1], state);
    render_agent_table(frame, chunks[2], state);
    render_custom_agents_table(frame, chunks[3], state);
    render_hints(frame, chunks[4], state);
}

fn render_config_path_section(frame: &mut Frame, area: Rect, state: &OpenCodeState) {
    let path_str = state
        .config_path
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "Not found".to_string());
    let exists_label = if state.config_exists {
        Span::styled(" [exists]", Style::default().fg(Color::Green))
    } else {
        Span::styled(" [not found]", Style::default().fg(Color::Red))
    };

    let lines = vec![
        Line::from(vec![
            Span::styled(
                " Config Path: ",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(path_str, Style::default().fg(Color::White)),
            exists_label,
        ]),
        Line::from(Span::styled(
            " Press c to set a custom path",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            " Config Path ",
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(Color::Cyan),
        ));
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_provider_section(frame: &mut Frame, area: Rect, state: &OpenCodeState) {
    let (label, color) = if state.provider_enabled {
        ("Enabled", Color::Green)
    } else {
        ("Disabled", Color::Red)
    };

    let mut lines = vec![Line::from(vec![
        Span::styled(
            " CodeRouter Provider: ",
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            label,
            Style::default()
                .fg(color)
                .add_modifier(Modifier::BOLD),
        ),
    ])];

    if state.provider_enabled {
        lines.push(Line::from(Span::styled(
            " Provider block injected into opencode.json",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            " Press t to enable, s to save",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            " Provider ",
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(Color::Cyan),
        ));
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_agent_table(frame: &mut Frame, area: Rect, state: &mut OpenCodeState) {
    let header_cells = ["Agent", "Assigned Group"];
    let header = Row::new(
        header_cells
            .iter()
            .map(|h| {
                Cell::from(*h).style(
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                )
            }),
    )
    .bottom_margin(1);

    let rows: Vec<Row> = state
        .agent_mappings
        .iter()
        .map(|m| {
            let group_display = m.group.as_deref().unwrap_or("—");
            let group_style = if m.group.is_some() {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            Row::new(vec![
                Cell::from(m.name.as_str()).style(Style::default().fg(Color::White)),
                Cell::from(group_display).style(group_style),
            ])
        })
        .collect();

    let mut table_state = TableState::default();
    if !state.agent_mappings.is_empty() {
        table_state.select(Some(state.selected_row));
    }

    let border_color = if state.mode == OpenCodeMode::List
        && state.focus_section == FocusSection::MappingTable
    {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let table = Table::new(rows, [Constraint::Length(20), Constraint::Min(20)])
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .title(Span::styled(
                    " Agent Mapping ",
                    Style::default()
                        .add_modifier(Modifier::BOLD)
                        .fg(Color::Cyan),
                )),
        )
        .row_highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        );

    frame.render_stateful_widget(table, area, &mut table_state);
}

fn render_custom_agents_table(frame: &mut Frame, area: Rect, state: &mut OpenCodeState) {
    let header_cells = ["Name", "Mode", "Group", "Enabled", "Description"];
    let header = Row::new(
        header_cells
            .iter()
            .map(|h| {
                Cell::from(*h).style(
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                )
            }),
    )
    .bottom_margin(1);

    let rows: Vec<Row> = state
        .custom_agents
        .iter()
        .map(|a| {
            let mode_str = match a.mode {
                custom_agents::AgentMode::Subagent => "subagent",
                custom_agents::AgentMode::All => "all",
                custom_agents::AgentMode::Primary => "primary",
            };
            let group_str = a.model.as_deref().unwrap_or("—");
            let enabled = if a.disable.unwrap_or(false) {
                Span::styled("disabled", Style::default().fg(Color::Red))
            } else {
                Span::styled("enabled", Style::default().fg(Color::Green))
            };
            let desc = if a.description.len() > 30 {
                format!("{}...", &a.description[..27])
            } else {
                a.description.clone()
            };
            Row::new(vec![
                Cell::from(a.name.as_str()).style(Style::default().fg(Color::White)),
                Cell::from(mode_str).style(Style::default().fg(Color::DarkGray)),
                Cell::from(group_str).style(Style::default().fg(Color::DarkGray)),
                Cell::from(enabled),
                Cell::from(desc).style(Style::default().fg(Color::DarkGray)),
            ])
        })
        .collect();

    let mut table_state = TableState::default();
    if !state.custom_agents.is_empty() {
        table_state.select(Some(state.custom_agent_selected));
    }

    let border_color = if state.mode == OpenCodeMode::List
        && state.focus_section == FocusSection::CustomAgents
    {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let count_label = format!(" Custom Agents ({}) ", state.custom_agents.len());
    let table = Table::new(
        rows,
        [
            Constraint::Length(16),
            Constraint::Length(10),
            Constraint::Length(12),
            Constraint::Length(10),
            Constraint::Min(16),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .title(Span::styled(
                count_label,
                Style::default()
                    .add_modifier(Modifier::BOLD)
                    .fg(Color::Cyan),
            )),
    )
    .row_highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    );

    frame.render_stateful_widget(table, area, &mut table_state);
}

fn render_hints(frame: &mut Frame, area: Rect, state: &OpenCodeState) {
    let hints = match state.mode {
        OpenCodeMode::List => match state.focus_section {
            FocusSection::MappingTable => {
                " j/k:Nav  Tab:Agents  t:Toggle  c:Path  Enter:Edit  s:Save  ?:Help "
            }
            FocusSection::CustomAgents => {
                " j/k:Nav  Tab:Mapping  a:Add  e:Edit  d:Del  Enter:Detail  ?:Help "
            }
        },
        _ => "",
    };
    let hint = Paragraph::new(Span::styled(
        hints,
        Style::default().fg(Color::DarkGray),
    ));
    frame.render_widget(hint, area);
}

fn render_mapping_popup(frame: &mut Frame, area: Rect, state: &OpenCodeState) {
    let idx = match state.mode {
        OpenCodeMode::EditMapping(i) => i,
        _ => return,
    };
    let agent_name = state
        .agent_mappings
        .get(idx)
        .map(|m| m.name.as_str())
        .unwrap_or("?");

    let group_count = state.available_groups.len();
    let height = (group_count + 5).min(20).min(area.height as usize) as u16;
    let width = 44.min(area.width);
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup_area = Rect::new(x, y, width, height);

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(format!(" Assign Group: {} ", agent_name))
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let inner_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);

    let none_style = if state.mapping_popup_idx == 0 {
        Style::default()
            .fg(Color::White)
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            " (none) — clear assignment",
            none_style,
        ))),
        inner_layout[0],
    );

    let visible_height = inner_layout[1].height as usize;
    let scroll = if state.mapping_popup_idx > visible_height {
        state.mapping_popup_idx - visible_height
    } else if state.mapping_popup_idx > 0 && state.mapping_popup_idx >= group_count + 1 {
        state.mapping_popup_idx.saturating_sub(visible_height)
    } else {
        0
    };

    let group_lines: Vec<Line> = state
        .available_groups
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible_height)
        .map(|(i, g)| {
            let popup_idx = i + 1;
            let style = if popup_idx == state.mapping_popup_idx {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            Line::from(Span::styled(format!(" {}", g), style))
        })
        .collect();
    frame.render_widget(Paragraph::new(group_lines), inner_layout[1]);

    let hint = " j/k:Select  Enter:Apply  Esc:Cancel ";
    frame.render_widget(
        Paragraph::new(Span::styled(
            hint,
            Style::default().fg(Color::DarkGray),
        ))
        .alignment(Alignment::Center),
        inner_layout[2],
    );
}

fn render_custom_path_popup(frame: &mut Frame, area: Rect, state: &mut OpenCodeState) {
    let width = 64.min(area.width);
    let height = 9.min(area.height);
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup_area = Rect::new(x, y, width, height);

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" Set Custom Config Path ")
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
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Span::styled(
            " Enter path to opencode.json:",
            Style::default().fg(Color::DarkGray),
        )),
        layout[0],
    );

    state.custom_path_input.set_block(
        Block::default()
            .title(" Path ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    frame.render_widget(&state.custom_path_input, layout[1]);

    frame.render_widget(
        Paragraph::new(Span::styled(
            " Enter:Save  Esc:Cancel ",
            Style::default().fg(Color::DarkGray),
        ))
        .alignment(Alignment::Center),
        layout[2],
    );
}

fn render_template_select_popup(frame: &mut Frame, area: Rect, state: &OpenCodeState) {
    let templates = custom_agents::get_templates();
    let total = templates.len() + 1;
    let height = (total + 4).min(18).min(area.height as usize) as u16;
    let width = 56.min(area.width);
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);

    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Select Template ")
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Black));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);

    let visible = chunks[0].height as usize;
    let scroll = if state.template_selected >= visible {
        state.template_selected - visible + 1
    } else {
        0
    };

    let mut lines: Vec<Line> = Vec::new();

    let blank_style = if state.template_selected == 0 {
        Style::default()
            .fg(Color::White)
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    lines.push(Line::from(Span::styled(" (blank) Start from scratch", blank_style)));

    for (i, t) in templates.iter().enumerate().skip(scroll).take(visible) {
        let idx = i + 1;
        let style = if idx == state.template_selected {
            Style::default()
                .fg(Color::White)
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        let desc = if t.description.len() > 38 {
            format!("{}...", &t.description[..35])
        } else {
            t.description.clone()
        };
        lines.push(Line::from(Span::styled(format!(" {} — {}", t.name, desc), style)));
    }

    frame.render_widget(Paragraph::new(lines), chunks[0]);
    frame.render_widget(
        Paragraph::new(Span::styled(
            " j/k:Nav  Enter:Select  Esc:Cancel ",
            Style::default().fg(Color::DarkGray),
        ))
        .alignment(Alignment::Center),
        chunks[1],
    );
}

fn render_agent_form_popup(frame: &mut Frame, area: Rect, state: &mut OpenCodeState) {
    let form = match &mut state.agent_form {
        Some(f) => f,
        None => return,
    };

    let is_edit = form.original_name.is_some();
    let title = if is_edit { " Edit Agent " } else { " Add Agent " };

    let width = 60.min(area.width);
    let height = 19.min(area.height);
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);

    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title(title)
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Black));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // name
            Constraint::Length(3),  // desc
            Constraint::Length(3),  // prompt
            Constraint::Length(1),  // mode+group
            Constraint::Length(1),  // steps+topp
            Constraint::Length(1),  // color
            Constraint::Length(1),  // hidden+disabled
            Constraint::Length(1),  // perm header
            Constraint::Length(1),  // edit+bash
            Constraint::Length(1),  // web+task
            Constraint::Length(1),  // hints
        ])
        .split(inner);

    let focused_border = Style::default().fg(Color::Cyan);
    let unfocused_border = Style::default().fg(Color::DarkGray);

    form.name.set_block(
        Block::default()
            .title(" Name ")
            .borders(Borders::ALL)
            .border_style(if form.focused == 0 {
                focused_border
            } else {
                unfocused_border
            }),
    );
    frame.render_widget(&form.name, chunks[0]);

    form.description.set_block(
        Block::default()
            .title(" Description ")
            .borders(Borders::ALL)
            .border_style(if form.focused == 1 {
                focused_border
            } else {
                unfocused_border
            }),
    );
    frame.render_widget(&form.description, chunks[1]);

    form.prompt.set_block(
        Block::default()
            .title(" Prompt ")
            .borders(Borders::ALL)
            .border_style(if form.focused == 2 {
                focused_border
            } else {
                unfocused_border
            }),
    );
    frame.render_widget(&form.prompt, chunks[2]);

    let mode_val = MODE_OPTIONS[form.mode_idx];
    let group_val = if form.group_idx == 0 {
        "none".to_string()
    } else {
        state
            .available_groups
            .get(form.group_idx - 1)
            .cloned()
            .unwrap_or_else(|| "none".to_string())
    };
    let mode_style = if form.focused == 3 {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let group_style = if form.focused == 4 {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Mode: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{} ◄►", mode_val), mode_style),
            Span::raw("   "),
            Span::styled("Group: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{} ◄►", group_val), group_style),
        ])),
        chunks[3],
    );

    let steps_style = if form.focused == 5 {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let topp_style = if form.focused == 6 {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let steps_display = if form.steps_buf.is_empty() {
        "—".to_string()
    } else {
        form.steps_buf.clone()
    };
    let topp_display = if form.top_p_buf.is_empty() {
        "—".to_string()
    } else {
        form.top_p_buf.clone()
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Steps: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("[{}]", steps_display),
                steps_style,
            ),
            Span::raw("   "),
            Span::styled("Top P: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("[{}]", topp_display), topp_style),
        ])),
        chunks[4],
    );

    let color_style = if form.focused == 7 {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let color_display = if form.color_buf.is_empty() {
        "—".to_string()
    } else {
        form.color_buf.clone()
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Color: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("[{}]", color_display), color_style),
        ])),
        chunks[5],
    );

    let hidden_mark = if form.hidden { "[x]" } else { "[ ]" };
    let disabled_mark = if form.disabled { "[x]" } else { "[ ]" };
    let hidden_style = if form.focused == 8 {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let disabled_style = if form.focused == 9 {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!(" {} Hidden ", hidden_mark), hidden_style),
            Span::raw("   "),
            Span::styled(format!(" {} Disabled ", disabled_mark), disabled_style),
        ])),
        chunks[6],
    );

    frame.render_widget(
        Paragraph::new(Span::styled(
            "── Permissions ──",
            Style::default().fg(Color::DarkGray),
        )),
        chunks[7],
    );

    let pe = PERM_OPTIONS[form.perm_edit];
    let pb = PERM_OPTIONS[form.perm_bash];
    let pe_style = if form.focused == 10 {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let pb_style = if form.focused == 11 {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Edit: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{} ◄►", pe), pe_style),
            Span::raw("   "),
            Span::styled("Bash: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{} ◄►", pb), pb_style),
        ])),
        chunks[8],
    );

    let pw = PERM_OPTIONS[form.perm_webfetch];
    let pt = PERM_OPTIONS[form.perm_task];
    let pw_style = if form.focused == 12 {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let pt_style = if form.focused == 13 {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Web: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{} ◄►", pw), pw_style),
            Span::raw("   "),
            Span::styled("Task: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{} ◄►", pt), pt_style),
        ])),
        chunks[9],
    );

    frame.render_widget(
        Paragraph::new(Span::styled(
            " Tab:Next  j/k:Change  Space:Toggle  Enter:Save  Esc:Cancel",
            Style::default().fg(Color::DarkGray),
        ))
        .alignment(Alignment::Center),
        chunks[10],
    );
}

fn render_agent_delete_popup(frame: &mut Frame, area: Rect, state: &OpenCodeState) {
    let idx = match state.mode {
        OpenCodeMode::AgentDeleteConfirm(i) => i,
        _ => return,
    };
    let name = state
        .custom_agents
        .get(idx)
        .map(|a| a.name.as_str())
        .unwrap_or("?");
    let msg = format!("Delete agent '{}'?", name);

    let width = (msg.len() as u16 + 6).min(area.width);
    let height = 5.min(area.height);
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);

    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Confirm Delete ")
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .style(Style::default().bg(Color::Black));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let text_chunks = Layout::default()
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
        text_chunks[0],
    );
    frame.render_widget(
        Paragraph::new(""),
        text_chunks[1],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            " y:Confirm  Esc:Cancel ",
            Style::default().fg(Color::DarkGray),
        ))
        .alignment(Alignment::Center),
        text_chunks[2],
    );
}

fn render_agent_detail(frame: &mut Frame, area: Rect, state: &mut OpenCodeState) {
    let idx = match state.mode {
        OpenCodeMode::AgentDetail(i) => i,
        _ => return,
    };
    let agent = match state.custom_agents.get(idx) {
        Some(a) => a.clone(),
        None => {
            frame.render_widget(
                Paragraph::new("Agent not found. Press Esc to go back.")
                    .style(Style::default().fg(Color::Red)),
                area,
            );
            return;
        }
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(12),
            Constraint::Min(4),
            Constraint::Length(1),
        ])
        .split(area);

    let mode_str = format!("{}", agent.mode);
    let group_str = agent.model.as_deref().unwrap_or("—");
    let steps_str = agent
        .steps
        .map(|s| s.to_string())
        .unwrap_or_else(|| "—".to_string());
    let topp_str = agent
        .top_p
        .map(|p| p.to_string())
        .unwrap_or_else(|| "—".to_string());
    let color_str = agent.color.as_deref().unwrap_or("—");
    let hidden_str = if agent.hidden.unwrap_or(false) {
        "yes"
    } else {
        "no"
    };
    let disabled_str = if agent.disable.unwrap_or(false) {
        "yes"
    } else {
        "no"
    };

    let mut info_lines = vec![
        Line::from(vec![
            Span::styled(
                format!(" {} ", agent.name),
                Style::default()
                    .add_modifier(Modifier::BOLD)
                    .fg(Color::Cyan),
            ),
        ]),
        Line::from(vec![
            Span::styled(" Description: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&agent.description, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled(" Mode: ", Style::default().fg(Color::DarkGray)),
            Span::styled(mode_str, Style::default().fg(Color::White)),
            Span::raw("  "),
            Span::styled("Group: ", Style::default().fg(Color::DarkGray)),
            Span::styled(group_str, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled(" Steps: ", Style::default().fg(Color::DarkGray)),
            Span::styled(steps_str, Style::default().fg(Color::White)),
            Span::raw("  "),
            Span::styled("Top P: ", Style::default().fg(Color::DarkGray)),
            Span::styled(topp_str, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled(" Color: ", Style::default().fg(Color::DarkGray)),
            Span::styled(color_str, Style::default().fg(Color::White)),
            Span::raw("  "),
            Span::styled("Hidden: ", Style::default().fg(Color::DarkGray)),
            Span::styled(hidden_str, Style::default().fg(Color::White)),
            Span::raw("  "),
            Span::styled("Disabled: ", Style::default().fg(Color::DarkGray)),
            Span::styled(disabled_str, Style::default().fg(Color::White)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            " Permissions:",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    if let Some(perm) = &agent.permission {
        let edit_str = perm
            .edit
            .as_ref()
            .map(|p| p.as_str())
            .unwrap_or("none");
        let bash_str = match &perm.bash {
            Some(custom_agents::BashPermission::Simple(p)) => p.as_str(),
            Some(custom_agents::BashPermission::Commands(map)) => {
                if map.is_empty() {
                    "none"
                } else {
                    "custom"
                }
            }
            None => "none",
        };
        let web_str = perm
            .webfetch
            .as_ref()
            .map(|p| p.as_str())
            .unwrap_or("none");
        let task_str = if let Some(map) = &perm.task {
            if map.is_empty() {
                "none"
            } else {
                "custom"
            }
        } else {
            "none"
        };
        info_lines.push(Line::from(vec![
            Span::styled("   Edit: ", Style::default().fg(Color::DarkGray)),
            Span::styled(edit_str, Style::default().fg(Color::White)),
            Span::raw("  "),
            Span::styled("Bash: ", Style::default().fg(Color::DarkGray)),
            Span::styled(bash_str, Style::default().fg(Color::White)),
            Span::raw("  "),
            Span::styled("Web: ", Style::default().fg(Color::DarkGray)),
            Span::styled(web_str, Style::default().fg(Color::White)),
            Span::raw("  "),
            Span::styled("Task: ", Style::default().fg(Color::DarkGray)),
            Span::styled(task_str, Style::default().fg(Color::White)),
        ]));
    } else {
        info_lines.push(Line::from(Span::styled(
            "   (none)",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let info_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    frame.render_widget(
        Paragraph::new(info_lines).block(info_block),
        chunks[0],
    );

    let visible_height = chunks[1].height.saturating_sub(2) as usize;
    let prompt_lines: Vec<Line> = agent
        .prompt
        .lines()
        .skip(state.custom_agent_scroll)
        .take(visible_height)
        .map(|l| Line::from(Span::styled(l.to_string(), Style::default().fg(Color::White))))
        .collect();
    let total_lines = agent.prompt.lines().count();
    let max_scroll = total_lines.saturating_sub(visible_height);
    state.custom_agent_scroll = state.custom_agent_scroll.min(max_scroll);

    let prompt_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            " Prompt ",
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(Color::Cyan),
        ));
    frame.render_widget(
        Paragraph::new(prompt_lines)
            .block(prompt_block)
            .wrap(Wrap { trim: false }),
        chunks[1],
    );

    let scroll_info = if total_lines > visible_height {
        format!(
            " [{}/{}] ",
            state.custom_agent_scroll + 1,
            max_scroll + 1
        )
    } else {
        String::new()
    };
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" Esc:Back  j/k:Scroll{}", scroll_info),
            Style::default().fg(Color::DarkGray),
        )),
        chunks[2],
    );
}

pub fn is_form_active() -> bool {
    if let Some(state_ref) = STATE.get() {
        if let Ok(guard) = state_ref.lock() {
            if let Some(state) = guard.as_ref() {
                return matches!(
                    state.mode,
                    OpenCodeMode::EditMapping(_)
                        | OpenCodeMode::SetCustomPath
                        | OpenCodeMode::AgentForm(_)
                );
            }
        }
    }
    false
}

fn handle_list_key(app: &mut App, key: KeyEvent, state: &mut OpenCodeState) {
    match key.code {
        KeyCode::Tab => {
            state.focus_section = match state.focus_section {
                FocusSection::MappingTable => FocusSection::CustomAgents,
                FocusSection::CustomAgents => FocusSection::MappingTable,
            };
        }
        KeyCode::Char('j') | KeyCode::Down => match state.focus_section {
            FocusSection::MappingTable => {
                if !state.agent_mappings.is_empty() {
                    state.selected_row =
                        (state.selected_row + 1).min(state.agent_mappings.len() - 1);
                }
            }
            FocusSection::CustomAgents => {
                if !state.custom_agents.is_empty() {
                    state.custom_agent_selected =
                        (state.custom_agent_selected + 1).min(state.custom_agents.len() - 1);
                }
            }
        },
        KeyCode::Char('k') | KeyCode::Up => match state.focus_section {
            FocusSection::MappingTable => {
                state.selected_row = state.selected_row.saturating_sub(1);
            }
            FocusSection::CustomAgents => {
                state.custom_agent_selected = state.custom_agent_selected.saturating_sub(1);
            }
        },
        KeyCode::Char('t') => {
            state.provider_enabled = !state.provider_enabled;
            let label = if state.provider_enabled {
                "enabled"
            } else {
                "disabled"
            };
            app.toast = Some(ToastMessage::new(format!(
                "Provider {} (press s to save)",
                label
            )));
        }
        KeyCode::Char('c') => {
            let current = state
                .config_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            state.custom_path_input = make_textarea(&current);
            state.mode = OpenCodeMode::SetCustomPath;
        }
        KeyCode::Char('s') => {
            do_save(app, state);
        }
        KeyCode::Enter => match state.focus_section {
            FocusSection::MappingTable => {
                if !state.agent_mappings.is_empty() {
                    let current_group = state
                        .agent_mappings
                        .get(state.selected_row)
                        .and_then(|m| m.group.as_deref());
                    state.mapping_popup_idx = if let Some(g) = current_group {
                        state
                            .available_groups
                            .iter()
                            .position(|ag| ag == g)
                            .map(|pos| pos + 1)
                            .unwrap_or(0)
                    } else {
                        0
                    };
                    state.mode = OpenCodeMode::EditMapping(state.selected_row);
                }
            }
            FocusSection::CustomAgents => {
                if !state.custom_agents.is_empty() {
                    state.custom_agent_scroll = 0;
                    state.mode = OpenCodeMode::AgentDetail(state.custom_agent_selected);
                }
            }
        },
        KeyCode::Char('a') => match state.focus_section {
            FocusSection::CustomAgents => {
                state.template_selected = 0;
                state.mode = OpenCodeMode::TemplateSelect;
            }
            _ => {}
        },
        KeyCode::Char('e') => match state.focus_section {
            FocusSection::CustomAgents => {
                if let Some(agent) = state.custom_agents.get(state.custom_agent_selected).cloned()
                {
                    let groups = state.available_groups.clone();
                    state.agent_form = Some(AgentFormState::from_agent(&agent, &groups));
                    state.mode = OpenCodeMode::AgentForm(Some(agent.name.clone()));
                }
            }
            _ => {}
        },
        KeyCode::Char('d') => match state.focus_section {
            FocusSection::CustomAgents => {
                if !state.custom_agents.is_empty() {
                    state.mode = OpenCodeMode::AgentDeleteConfirm(state.custom_agent_selected);
                }
            }
            _ => {}
        },
        _ => {}
    }
}

fn handle_mapping_key(_app: &mut App, key: KeyEvent, state: &mut OpenCodeState) {
    let max_idx = state.available_groups.len();
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            state.mapping_popup_idx = (state.mapping_popup_idx + 1).min(max_idx);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.mapping_popup_idx = state.mapping_popup_idx.saturating_sub(1);
        }
        KeyCode::Enter => {
            if let OpenCodeMode::EditMapping(idx) = state.mode {
                if idx < state.agent_mappings.len() {
                    if state.mapping_popup_idx == 0 {
                        state.agent_mappings[idx].group = None;
                    } else {
                        let gi = state.mapping_popup_idx - 1;
                        if gi < state.available_groups.len() {
                            state.agent_mappings[idx].group =
                                Some(state.available_groups[gi].clone());
                        }
                    }
                }
            }
            state.mode = OpenCodeMode::List;
        }
        KeyCode::Esc => {
            state.mode = OpenCodeMode::List;
        }
        _ => {}
    }
}

fn handle_custom_path_key(app: &mut App, key: KeyEvent, state: &mut OpenCodeState) {
    match key.code {
        KeyCode::Enter => {
            let path = get_ta_val(&state.custom_path_input);
            if path.is_empty() {
                state.mode = OpenCodeMode::List;
                return;
            }
            match config_writer::save_opencode_config_path(&path) {
                Ok(()) => {
                    app.toast =
                        Some(ToastMessage::new(format!("Config path set to: {}", path)));
                    state.reload();
                }
                Err(e) => {
                    app.toast = Some(ToastMessage::new(format!("Error: {}", e)));
                }
            }
            state.mode = OpenCodeMode::List;
        }
        KeyCode::Esc => {
            state.mode = OpenCodeMode::List;
        }
        _ => {
            state.custom_path_input.input(key);
        }
    }
}

fn handle_template_select_key(app: &mut App, key: KeyEvent, state: &mut OpenCodeState) {
    let templates = custom_agents::get_templates();
    let max = templates.len() + 1;
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            state.template_selected = (state.template_selected + 1).min(max - 1);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.template_selected = state.template_selected.saturating_sub(1);
        }
        KeyCode::Enter => {
            if state.template_selected == 0 {
                state.agent_form = Some(AgentFormState::new_blank());
            } else if let Some(tmpl) = templates.get(state.template_selected - 1) {
                state.agent_form = Some(AgentFormState::from_template(tmpl));
            }
            state.mode = OpenCodeMode::AgentForm(None);
        }
        KeyCode::Esc => {
            state.mode = OpenCodeMode::List;
        }
        _ => {}
    }
    let _ = app;
}

fn handle_agent_form_key(app: &mut App, key: KeyEvent, state: &mut OpenCodeState) {
    let form = match &mut state.agent_form {
        Some(f) => f,
        None => {
            state.mode = OpenCodeMode::List;
            return;
        }
    };

    match key.code {
        KeyCode::Tab => {
            form.focused = (form.focused + 1) % FORM_FIELD_COUNT;
        }
        KeyCode::BackTab => {
            form.focused = if form.focused == 0 {
                FORM_FIELD_COUNT - 1
            } else {
                form.focused - 1
            };
        }
        KeyCode::Enter => {
            submit_agent_form(app, state);
        }
        KeyCode::Esc => {
            state.agent_form = None;
            state.mode = OpenCodeMode::List;
        }
        _ => {
            match form.focused {
                0 => {
                    form.name.input(key);
                }
                1 => {
                    form.description.input(key);
                }
                2 => {
                    form.prompt.input(key);
                }
                3 => match key.code {
                    KeyCode::Char('j') | KeyCode::Down | KeyCode::Char(' ') => {
                        form.mode_idx = (form.mode_idx + 1) % MODE_OPTIONS.len();
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        form.mode_idx = if form.mode_idx == 0 {
                            MODE_OPTIONS.len() - 1
                        } else {
                            form.mode_idx - 1
                        };
                    }
                    _ => {}
                },
                4 => {
                    let max_group = state.available_groups.len() + 1;
                    match key.code {
                        KeyCode::Char('j') | KeyCode::Down | KeyCode::Char(' ') => {
                            form.group_idx = (form.group_idx + 1) % max_group;
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            form.group_idx = if form.group_idx == 0 {
                                max_group - 1
                            } else {
                                form.group_idx - 1
                            };
                        }
                        _ => {}
                    }
                }
                5 => match key.code {
                    KeyCode::Char(c) if c.is_ascii_digit() => {
                        form.steps_buf.push(c);
                    }
                    KeyCode::Backspace => {
                        form.steps_buf.pop();
                    }
                    _ => {}
                },
                6 => match key.code {
                    KeyCode::Char(c) if c.is_ascii_digit() || c == '.' => {
                        if c == '.' && form.top_p_buf.contains('.') {
                            return;
                        }
                        form.top_p_buf.push(c);
                    }
                    KeyCode::Backspace => {
                        form.top_p_buf.pop();
                    }
                    _ => {}
                },
                7 => match key.code {
                    KeyCode::Char(c) if !c.is_control() => {
                        form.color_buf.push(c);
                    }
                    KeyCode::Backspace => {
                        form.color_buf.pop();
                    }
                    _ => {}
                },
                8 => match key.code {
                    KeyCode::Char(' ') | KeyCode::Enter => {
                        form.hidden = !form.hidden;
                    }
                    _ => {}
                },
                9 => match key.code {
                    KeyCode::Char(' ') | KeyCode::Enter => {
                        form.disabled = !form.disabled;
                    }
                    _ => {}
                },
                10 => match key.code {
                    KeyCode::Char('j') | KeyCode::Down | KeyCode::Char(' ') => {
                        form.perm_edit = (form.perm_edit + 1) % PERM_OPTIONS.len();
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        form.perm_edit = if form.perm_edit == 0 {
                            PERM_OPTIONS.len() - 1
                        } else {
                            form.perm_edit - 1
                        };
                    }
                    _ => {}
                },
                11 => match key.code {
                    KeyCode::Char('j') | KeyCode::Down | KeyCode::Char(' ') => {
                        form.perm_bash = (form.perm_bash + 1) % PERM_OPTIONS.len();
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        form.perm_bash = if form.perm_bash == 0 {
                            PERM_OPTIONS.len() - 1
                        } else {
                            form.perm_bash - 1
                        };
                    }
                    _ => {}
                },
                12 => match key.code {
                    KeyCode::Char('j') | KeyCode::Down | KeyCode::Char(' ') => {
                        form.perm_webfetch = (form.perm_webfetch + 1) % PERM_OPTIONS.len();
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        form.perm_webfetch = if form.perm_webfetch == 0 {
                            PERM_OPTIONS.len() - 1
                        } else {
                            form.perm_webfetch - 1
                        };
                    }
                    _ => {}
                },
                13 => match key.code {
                    KeyCode::Char('j') | KeyCode::Down | KeyCode::Char(' ') => {
                        form.perm_task = (form.perm_task + 1) % PERM_OPTIONS.len();
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        form.perm_task = if form.perm_task == 0 {
                            PERM_OPTIONS.len() - 1
                        } else {
                            form.perm_task - 1
                        };
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
}

fn handle_agent_delete_key(app: &mut App, key: KeyEvent, state: &mut OpenCodeState) {
    match key.code {
        KeyCode::Char('y') => {
            let idx = match state.mode {
                OpenCodeMode::AgentDeleteConfirm(i) => i,
                _ => return,
            };
            if let Some(agent) = state.custom_agents.get(idx) {
                let name = agent.name.clone();
                match custom_agents::delete_agent(&name) {
                    Ok(()) => {
                        app.toast = Some(ToastMessage::new(format!("Deleted '{}'", name)));
                        state.custom_agents = load_custom_agents();
                        if state.custom_agent_selected >= state.custom_agents.len()
                            && !state.custom_agents.is_empty()
                        {
                            state.custom_agent_selected = state.custom_agents.len() - 1;
                        }
                    }
                    Err(e) => {
                        app.toast = Some(ToastMessage::new(format!("Error: {}", e)));
                    }
                }
            }
            state.mode = OpenCodeMode::List;
        }
        KeyCode::Esc => {
            state.mode = OpenCodeMode::List;
        }
        _ => {}
    }
}

fn handle_agent_detail_key(_app: &mut App, key: KeyEvent, state: &mut OpenCodeState) {
    let idx = match state.mode {
        OpenCodeMode::AgentDetail(i) => i,
        _ => return,
    };
    let total_lines = state
        .custom_agents
        .get(idx)
        .map(|a| a.prompt.lines().count())
        .unwrap_or(0);

    match key.code {
        KeyCode::Esc => {
            state.mode = OpenCodeMode::List;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            state.custom_agent_scroll = state
                .custom_agent_scroll
                .saturating_add(1)
                .min(total_lines.saturating_sub(1));
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.custom_agent_scroll = state.custom_agent_scroll.saturating_sub(1);
        }
        _ => {}
    }
}

fn submit_agent_form(app: &mut App, state: &mut OpenCodeState) {
    let form = match state.agent_form.take() {
        Some(f) => f,
        None => {
            state.mode = OpenCodeMode::List;
            return;
        }
    };

    let agent = form.to_agent(&state.available_groups);

    if agent.name.is_empty() {
        app.toast = Some(ToastMessage::new("Name is required"));
        state.agent_form = Some(form);
        return;
    }
    if agent.description.is_empty() {
        app.toast = Some(ToastMessage::new("Description is required"));
        state.agent_form = Some(form);
        return;
    }
    if agent.prompt.is_empty() {
        app.toast = Some(ToastMessage::new("Prompt is required"));
        state.agent_form = Some(form);
        return;
    }

    let result = match &form.original_name {
        Some(orig_name) => custom_agents::update_agent(orig_name, &agent),
        None => custom_agents::create_agent(&agent),
    };

    match result {
        Ok(_) => {
            let label = if form.original_name.is_some() {
                "updated"
            } else {
                "created"
            };
            app.toast = Some(ToastMessage::new(format!(
                "Agent '{}' {}",
                agent.name, label
            )));
            state.custom_agents = load_custom_agents();
            state.mode = OpenCodeMode::List;
        }
        Err(e) => {
            app.toast = Some(ToastMessage::new(format!("Error: {}", e)));
            state.agent_form = Some(form);
        }
    }
}

fn do_save(app: &mut App, state: &mut OpenCodeState) {
    let config_path = match &state.config_path {
        Some(p) => p.clone(),
        None => {
            app.toast = Some(ToastMessage::new("No config path set"));
            return;
        }
    };

    if state.provider_enabled {
        let groups = store::load_groups().unwrap_or_default();
        let providers = store::load_providers().unwrap_or_default();
        let app_config = store::load_app_config().unwrap_or_default();
        if let Err(e) = config_writer::inject_provider(
            &config_path,
            &groups,
            &providers,
            app_config.proxy_port,
            &HashMap::new(),
        ) {
            app.toast =
                Some(ToastMessage::new(format!("Error injecting provider: {}", e)));
            return;
        }
    } else {
        if let Err(e) = config_writer::remove_provider(&config_path) {
            app.toast =
                Some(ToastMessage::new(format!("Error removing provider: {}", e)));
            return;
        }
    }

    let mapping = config_writer::AgentMapping {
        build: state.agent_mappings.get(0).and_then(|m| m.group.clone()),
        plan: state.agent_mappings.get(1).and_then(|m| m.group.clone()),
        general: state.agent_mappings.get(2).and_then(|m| m.group.clone()),
        explore: state.agent_mappings.get(3).and_then(|m| m.group.clone()),
        compaction: state.agent_mappings.get(4).and_then(|m| m.group.clone()),
        title: state.agent_mappings.get(5).and_then(|m| m.group.clone()),
        summary: state.agent_mappings.get(6).and_then(|m| m.group.clone()),
        small_model: state.agent_mappings.get(7).and_then(|m| m.group.clone()),
    };

    if let Err(e) = config_writer::set_agent_models(&config_path, &mapping) {
        app.toast =
            Some(ToastMessage::new(format!("Error setting agent models: {}", e)));
        return;
    }

    app.toast = Some(ToastMessage::new("OpenCode config saved"));
    state.reload();
}
