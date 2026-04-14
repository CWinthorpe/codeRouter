//! Custom OpenCode agent management via markdown files.
//!
//! Agents are stored as `.md` files in `~/.config/opencode/agents/` with YAML
//! frontmatter containing configuration and a markdown body containing the
//! system prompt.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Permission level for a specific tool or command pattern.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PermissionLevel {
    Allow,
    Deny,
    Ask,
}

impl PermissionLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            PermissionLevel::Allow => "allow",
            PermissionLevel::Deny => "deny",
            PermissionLevel::Ask => "ask",
        }
    }
}

/// Bash permission: either a simple level or a map of command patterns to levels.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(untagged)]
pub enum BashPermission {
    Simple(PermissionLevel),
    Commands(HashMap<String, PermissionLevel>),
}

/// Tool access permissions for a custom agent.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct AgentPermissions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edit: Option<PermissionLevel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bash: Option<BashPermission>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webfetch: Option<PermissionLevel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<HashMap<String, PermissionLevel>>,
}

/// Agent mode determining how the agent can be invoked.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AgentMode {
    Primary,
    #[default]
    Subagent,
    #[serde(alias = "all")]
    All,
}

impl std::fmt::Display for AgentMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentMode::Primary => write!(f, "primary"),
            AgentMode::Subagent => write!(f, "subagent"),
            AgentMode::All => write!(f, "all"),
        }
    }
}

/// YAML frontmatter structure for custom agent markdown files.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct AgentFrontmatter {
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<AgentMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "top_p")]
    pub top_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<AgentPermissions>,
    /// Additional provider-specific options passed through as-is.
    #[serde(flatten, default)]
    pub additional: HashMap<String, serde_json::Value>,
}

/// A custom OpenCode agent defined via markdown file.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CustomAgent {
    /// Agent identifier (derived from filename, without .md extension).
    pub name: String,
    /// Brief description of what the agent does.
    pub description: String,
    /// How the agent can be used.
    #[serde(default)]
    pub mode: AgentMode,
    /// CodeRouter model group alias (written as coderouter/<alias>).
    #[serde(default)]
    pub model: Option<String>,
    /// System prompt / instructions (markdown body).
    #[serde(default)]
    pub prompt: String,
    /// Temperature for response generation (0.0-1.0).
    #[serde(default)]
    pub temperature: Option<f64>,
    /// Maximum agentic iterations.
    #[serde(default)]
    pub steps: Option<u64>,
    /// Whether the agent is disabled.
    #[serde(default)]
    pub disable: Option<bool>,
    /// Hide subagent from @ autocomplete.
    #[serde(default)]
    pub hidden: Option<bool>,
    /// Visual color in the UI.
    #[serde(default)]
    pub color: Option<String>,
    /// Top P for response diversity.
    #[serde(default)]
    pub top_p: Option<f64>,
    /// Tool access permissions.
    #[serde(default)]
    pub permission: Option<AgentPermissions>,
    /// Additional provider-specific options.
    #[serde(default)]
    pub additional: HashMap<String, serde_json::Value>,
}

/// A built-in template for creating a new custom agent.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AgentTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub icon: String,
    pub agent: TemplateAgent,
}

/// Pre-filled agent configuration from a template (excludes name).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct TemplateAgent {
    pub description: String,
    pub mode: AgentMode,
    pub prompt: String,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub steps: Option<u64>,
    #[serde(default)]
    pub hidden: Option<bool>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub top_p: Option<f64>,
    #[serde(default)]
    pub permission: Option<AgentPermissions>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub disable: Option<bool>,
    #[serde(default)]
    pub additional: HashMap<String, serde_json::Value>,
}

/// Returns the directory for global custom agents: `~/.config/opencode/agents/`.
pub fn agents_dir() -> Option<PathBuf> {
    let mut path = dirs::home_dir()?;
    path.push(".config/opencode/agents");
    Some(path)
}

/// Lists all custom agents by reading `.md` files from the agents directory.
///
/// Returns a vector of [`CustomAgent`] parsed from their markdown files.
/// Files that cannot be parsed are silently skipped.
pub fn list_agents() -> Result<Vec<CustomAgent>> {
    let dir = match agents_dir() {
        Some(d) => d,
        None => return Ok(Vec::new()),
    };
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut agents = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        match parse_agent_file(&path) {
            Ok(agent) => agents.push(agent),
            Err(e) => eprintln!("[custom_agents] Skipping {path:?}: {e}"),
        }
    }
    agents.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(agents)
}

fn find_unique_path(dir: &Path, base: &str, exclude: Option<&Path>) -> PathBuf {
    let path = dir.join(format!("{base}.md"));
    if !path.exists() || exclude == Some(path.as_path()) {
        return path;
    }
    for i in 1u32.. {
        let candidate = dir.join(format!("{base}-{i}.md"));
        if !candidate.exists() || exclude == Some(candidate.as_path()) {
            return candidate;
        }
    }
    unreachable!()
}

/// Resolves an agent's actual on-disk path by searching `list_agents()` for a
/// name match, rather than naively constructing a path from the sanitized name.
fn resolve_agent_path(name: &str) -> Result<PathBuf> {
    let dir = agents_dir().ok_or("Could not resolve home directory")?;
    let agents = list_agents()?;
    let agent = agents
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| format!("Agent '{name}' not found"))?;
    Ok(dir.join(format!("{}.md", agent.name)))
}

/// Creates a new custom agent by writing a markdown file.
///
/// The filename is derived from the agent name (lowercased, spaces replaced
/// with hyphens). Returns the path to the created file.
///
/// # Errors
/// Returns an error if an agent with the same name already exists, if the
/// agents directory cannot be created, or the file cannot be written.
pub fn create_agent(agent: &CustomAgent) -> Result<PathBuf> {
    let dir = agents_dir().ok_or("Could not resolve home directory")?;
    if !dir.exists() {
        fs::create_dir_all(&dir)?;
    }

    let sanitized = sanitize_filename(&agent.name);
    if sanitized.is_empty() {
        return Err("Agent name is empty after sanitization".into());
    }

    if agent.description.trim().is_empty() {
        return Err("Agent description must not be empty".into());
    }
    if agent.prompt.trim().is_empty() {
        return Err("Agent prompt must not be empty".into());
    }

    // Compare sanitized names so inputs differing only by characters that
    // normalize identically (e.g. "My Agent" vs "My-Agent") are caught.
    for existing in list_agents()? {
        if sanitize_filename(&existing.name) == sanitized {
            return Err(format!("Agent '{}' already exists", agent.name).into());
        }
    }

    let path = find_unique_path(&dir, &sanitized, None);
    write_agent_file(&path, agent)?;

    if let Err(e) = parse_agent_file(&path) {
        let _ = fs::remove_file(&path);
        return Err(format!("Failed to verify written agent file: {e}").into());
    }

    Ok(path)
}

/// Updates an existing custom agent by rewriting its markdown file.
///
/// # Errors
/// Returns an error if the original agent file does not exist, if the file
/// cannot be written, or if the old file cannot be removed during rename.
pub fn update_agent(name: &str, agent: &CustomAgent) -> Result<PathBuf> {
    let dir = agents_dir().ok_or("Could not resolve home directory")?;
    let old_path = resolve_agent_path(name)?;

    let new_sanitized = sanitize_filename(&agent.name);
    if new_sanitized.is_empty() {
        return Err("Agent name is empty after sanitization".into());
    }

    if agent.description.trim().is_empty() {
        return Err("Agent description must not be empty".into());
    }
    if agent.prompt.trim().is_empty() {
        return Err("Agent prompt must not be empty".into());
    }

    // Check for name conflicts with other agents (excluding the one being updated).
    for existing in list_agents()? {
        if existing.name.eq_ignore_ascii_case(name) {
            continue;
        }
        if sanitize_filename(&existing.name) == new_sanitized {
            return Err(format!("Agent '{}' already exists", agent.name).into());
        }
    }

    let new_path = find_unique_path(&dir, &new_sanitized, Some(&old_path));

    // Write new file first, then verify it can be parsed back.
    write_agent_file(&new_path, agent)?;

    if let Err(e) = parse_agent_file(&new_path) {
        // Clean up the new file; leave the old file intact.
        if new_path != old_path {
            let _ = fs::remove_file(&new_path);
        }
        return Err(format!("Failed to verify written agent file: {e}").into());
    }

    // Only delete the old file after successful parse verification.
    if old_path != new_path {
        fs::remove_file(&old_path).map_err(|e| {
            format!(
                "Failed to remove old agent file '{}': {}",
                old_path.display(),
                e
            )
        })?;
    }

    Ok(new_path)
}

/// Deletes a custom agent by removing its markdown file.
///
/// # Errors
/// Returns an error if the file does not exist or cannot be removed.
pub fn delete_agent(name: &str) -> Result<()> {
    let path = resolve_agent_path(name)?;
    fs::remove_file(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!("Agent '{name}' not found").into()
        } else {
            Box::new(e) as Box<dyn std::error::Error + Send + Sync>
        }
    })?;
    Ok(())
}

/// Returns the built-in agent templates available for selection.
pub fn get_templates() -> Vec<AgentTemplate> {
    vec![
        AgentTemplate {
            id: "code-reviewer".to_string(),
            name: "Code Reviewer".to_string(),
            description: "Reviews code for quality, security, and best practices".to_string(),
            icon: "Search".to_string(),
            agent: TemplateAgent {
                description: "Reviews code for quality, security, and best practices. Use this when you need a thorough code review before merging.".to_string(),
                mode: AgentMode::Subagent,
                temperature: Some(0.2),
                steps: None,
                hidden: None,
                color: Some("accent".to_string()),
                top_p: None,
                permission: Some(AgentPermissions {
                    edit: Some(PermissionLevel::Deny),
                    bash: Some(BashPermission::Commands(HashMap::from([
                        ("*".to_string(), PermissionLevel::Ask),
                        ("git diff*".to_string(), PermissionLevel::Allow),
                        ("git log*".to_string(), PermissionLevel::Allow),
                        ("grep *".to_string(), PermissionLevel::Allow),
                    ]))),
                    webfetch: Some(PermissionLevel::Deny),
                    task: None,
                }),
                prompt: "You are an expert code reviewer. Focus on:\n- Code quality and best practices\n- Potential bugs and edge cases\n- Performance implications\n- Security considerations\n- Maintainability and readability\n\nProvide constructive feedback without making direct changes. Reference specific lines or functions when pointing out issues.".to_string(),
                ..Default::default()
            },
        },
        AgentTemplate {
            id: "docs-writer".to_string(),
            name: "Documentation Writer".to_string(),
            description: "Writes and maintains project documentation".to_string(),
            icon: "FileText".to_string(),
            agent: TemplateAgent {
                description: "Writes clear, comprehensive documentation for codebases, APIs, and developer tools.".to_string(),
                mode: AgentMode::Subagent,
                temperature: Some(0.4),
                steps: None,
                hidden: None,
                color: Some("info".to_string()),
                top_p: None,
                permission: Some(AgentPermissions {
                    edit: Some(PermissionLevel::Allow),
                    bash: Some(BashPermission::Simple(PermissionLevel::Deny)),
                    webfetch: Some(PermissionLevel::Allow),
                    task: None,
                }),
                prompt: "You are a technical writer. Create clear, comprehensive documentation.\n\nFocus on:\n- Clear explanations with proper structure\n- Code examples that are accurate and tested\n- User-friendly language appropriate for the audience\n- Consistent formatting and style\n\nWrite documentation that helps developers understand and use the code effectively.".to_string(),
                ..Default::default()
            },
        },
        AgentTemplate {
            id: "security-auditor".to_string(),
            name: "Security Auditor".to_string(),
            description: "Performs security audits and identifies vulnerabilities".to_string(),
            icon: "Shield".to_string(),
            agent: TemplateAgent {
                description: "Performs security audits and identifies vulnerabilities in code and configurations.".to_string(),
                mode: AgentMode::Subagent,
                temperature: Some(0.1),
                steps: None,
                hidden: None,
                color: Some("error".to_string()),
                top_p: None,
                permission: Some(AgentPermissions {
                    edit: Some(PermissionLevel::Deny),
                    bash: Some(BashPermission::Simple(PermissionLevel::Deny)),
                    webfetch: Some(PermissionLevel::Allow),
                    task: None,
                }),
                prompt: "You are a security expert. Focus on identifying potential security issues.\n\nLook for:\n- Input validation vulnerabilities\n- Authentication and authorization flaws\n- Data exposure risks\n- Dependency vulnerabilities\n- Configuration security issues\n- Injection attacks (SQL, XSS, command injection)\n- Insecure cryptographic practices\n\nProvide severity ratings and specific remediation steps for each finding.".to_string(),
                ..Default::default()
            },
        },
        AgentTemplate {
            id: "debugger".to_string(),
            name: "Debugger".to_string(),
            description: "Investigates and diagnoses bugs with read and bash access".to_string(),
            icon: "Bug".to_string(),
            agent: TemplateAgent {
                description: "Focused on investigating and diagnosing bugs. Has read access to files and can run diagnostic commands.".to_string(),
                mode: AgentMode::Subagent,
                temperature: Some(0.2),
                steps: None,
                hidden: None,
                color: Some("warning".to_string()),
                top_p: None,
                permission: Some(AgentPermissions {
                    edit: Some(PermissionLevel::Deny),
                    bash: Some(BashPermission::Simple(PermissionLevel::Allow)),
                    webfetch: Some(PermissionLevel::Deny),
                    task: None,
                }),
                prompt: "You are a debugging expert. Investigate and diagnose issues systematically.\n\nApproach:\n1. Reproduce the issue if possible\n2. Read relevant code and logs\n3. Form hypotheses about root causes\n4. Test hypotheses using diagnostic commands\n5. Report findings with evidence\n\nDo not make code changes. Focus on identifying the root cause and recommending a fix strategy.".to_string(),
                ..Default::default()
            },
        },
        AgentTemplate {
            id: "test-writer".to_string(),
            name: "Test Writer".to_string(),
            description: "Writes comprehensive tests for code coverage".to_string(),
            icon: "CheckSquare".to_string(),
            agent: TemplateAgent {
                description: "Writes comprehensive unit, integration, and end-to-end tests for code coverage.".to_string(),
                mode: AgentMode::Subagent,
                temperature: Some(0.3),
                steps: None,
                hidden: None,
                color: Some("success".to_string()),
                top_p: None,
                permission: Some(AgentPermissions {
                    edit: Some(PermissionLevel::Allow),
                    bash: Some(BashPermission::Simple(PermissionLevel::Allow)),
                    webfetch: Some(PermissionLevel::Deny),
                    task: None,
                }),
                prompt: "You are a test engineering expert. Write comprehensive, maintainable tests.\n\nFocus on:\n- Unit tests for individual functions\n- Integration tests for component interactions\n- Edge cases and error conditions\n- Clear test names that describe the scenario\n- Proper use of mocks and fixtures\n\nEnsure tests are deterministic, fast, and provide clear failure messages.".to_string(),
                ..Default::default()
            },
        },
        AgentTemplate {
            id: "gas-optimizer".to_string(),
            name: "Gas Optimization Auditor".to_string(),
            description: "Reviews Ethereum smart contracts for gas optimization opportunities".to_string(),
            icon: "Zap".to_string(),
            agent: TemplateAgent {
                description: "A specialized agent for reviewing Ethereum smart contracts and identifying gas optimization opportunities. Use this when you need to analyze Solidity code for inefficiencies, suggest cheaper alternatives, or estimate gas savings.".to_string(),
                mode: AgentMode::Subagent,
                temperature: Some(0.2),
                steps: None,
                hidden: None,
                color: Some("#f59e0b".to_string()),
                top_p: None,
                permission: Some(AgentPermissions {
                    edit: Some(PermissionLevel::Deny),
                    bash: Some(BashPermission::Simple(PermissionLevel::Deny)),
                    webfetch: Some(PermissionLevel::Allow),
                    task: None,
                }),
                prompt: "You are an expert Ethereum smart contract gas optimization auditor with deep knowledge of the EVM, Solidity, and Yul/assembly optimizations.\n\nWhen reviewing contracts, analyze and report on the following:\n\n## Storage Optimizations\n- Slot packing (group variables to minimize storage slots)\n- Use immutable and constant where applicable\n- Avoid unnecessary SSTORE/SLOAD operations\n- Cache storage variables in memory within loops\n\n## Data Type Optimizations\n- Prefer uint256 over smaller uints unless packing\n- Use bytes32 over string for fixed-length data\n- Use calldata instead of memory for read-only function parameters\n\n## Control Flow Optimizations\n- Short-circuit conditions (cheapest checks first)\n- Avoid redundant checks and reverts\n- Use custom errors instead of revert strings\n\n## Loop Optimizations\n- Cache array lengths outside loops\n- Use unchecked blocks for safe arithmetic\n- Avoid dynamic array resizing inside loops\n\n## Architecture Optimizations\n- Suggest batching patterns where applicable\n- Identify unnecessary external calls\n- Flag redundant events or emits\n\n## Output Format\nFor each finding, provide:\n1. Location — function or line reference\n2. Issue — description of the inefficiency\n3. Recommendation — specific fix\n4. Estimated Gas Savings — rough estimate where possible".to_string(),
                ..Default::default()
            },
        },
    ]
}

/// Parses a markdown file with YAML frontmatter into a [`CustomAgent`].
///
/// The file format is:
/// ```text
/// ---
/// description: ...
/// mode: subagent
/// ...
/// ---
///
/// Prompt body here...
/// ```
pub fn parse_agent_file(path: &Path) -> Result<CustomAgent> {
    let contents = fs::read_to_string(path)?;
    let (frontmatter, prompt) = split_frontmatter(&contents)?;
    if frontmatter.trim().is_empty() {
        return Err("Frontmatter must contain at least a 'description' field".into());
    }
    let fm: AgentFrontmatter = serde_yaml::from_str(&frontmatter)?;
    if fm.description.trim().is_empty() {
        return Err("Frontmatter must contain at least a 'description' field".into());
    }
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    Ok(CustomAgent {
        name,
        description: fm.description,
        mode: fm.mode.unwrap_or_default(),
        model: fm.model.map(|m| {
            m.strip_prefix("coderouter/")
                .map(|s| s.to_string())
                .unwrap_or(m)
        }),
        prompt,
        temperature: fm.temperature,
        steps: fm.steps,
        disable: fm.disable,
        hidden: fm.hidden,
        color: fm.color,
        top_p: fm.top_p,
        permission: fm.permission,
        additional: fm.additional,
    })
}

/// Writes a [`CustomAgent`] to a markdown file with YAML frontmatter.
fn write_agent_file(path: &Path, agent: &CustomAgent) -> Result<()> {
    let model = agent.model.as_ref().map(|m| {
        if m.starts_with("coderouter/") {
            m.clone()
        } else {
            format!("coderouter/{}", m)
        }
    });

    let frontmatter = AgentFrontmatter {
        description: agent.description.clone(),
        mode: if agent.mode == AgentMode::default() {
            None
        } else {
            Some(agent.mode.clone())
        },
        model,
        temperature: agent.temperature,
        steps: agent.steps,
        disable: agent.disable.and_then(|v| v.then_some(true)),
        hidden: agent.hidden.and_then(|v| v.then_some(true)),
        color: agent.color.clone(),
        top_p: agent.top_p,
        permission: agent.permission.clone(),
        additional: agent.additional.clone(),
    };

    let yaml = serde_yaml::to_string(&frontmatter)?;
    let yaml = yaml.strip_prefix("---\n").unwrap_or(&yaml);
    let content = format!("---\n{}---\n\n{}", yaml, agent.prompt);
    let tmp_path = path.with_extension("md.tmp");
    fs::write(&tmp_path, &content)?;
    fs::rename(&tmp_path, path)?;
    Ok(())
}

/// Splits a markdown file into YAML frontmatter and body.
///
/// Expects the format `---\n...yaml...\n---\n...body...`.
/// Returns an error if the frontmatter delimiters are not found.
fn split_frontmatter(contents: &str) -> Result<(String, String)> {
    let normalized = contents.replace("\r\n", "\n");
    let trimmed = normalized.trim_start();
    if !trimmed.starts_with("---") {
        return Err("Missing frontmatter opening delimiter".into());
    }
    let rest = &trimmed[3..];
    let end = rest
        .find("\n---\n")
        .or_else(|| rest.find("\n---").filter(|&pos| pos + 4 == rest.len()))
        .ok_or("Missing frontmatter closing delimiter")?;
    let yaml = rest[..end].trim();
    let body = rest[end + 4..].trim();
    Ok((yaml.to_string(), body.to_string()))
}

/// Sanitizes an agent name into a safe filename: lowercase, hyphens for spaces,
/// alphanumeric characters and hyphens only.
fn sanitize_filename(name: &str) -> String {
    name.to_lowercase()
        .trim()
        .replace([' ', '_'], "-")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-')
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn test_dir() -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "coderouter_agents_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        path
    }

    #[test]
    fn test_split_frontmatter() {
        let input = "---\ndescription: Test agent\nmode: subagent\n---\n\nHello world";
        let (yaml, body) = split_frontmatter(input).unwrap();
        assert!(yaml.contains("description: Test agent"));
        assert!(yaml.contains("mode: subagent"));
        assert_eq!(body, "Hello world");
    }

    #[test]
    fn test_split_frontmatter_missing_opening() {
        let input = "description: Test\n---\n\nHello";
        assert!(split_frontmatter(input).is_err());
    }

    #[test]
    fn test_split_frontmatter_missing_closing() {
        let input = "---\ndescription: Test\n\nHello";
        assert!(split_frontmatter(input).is_err());
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("My Agent"), "my-agent");
        assert_eq!(sanitize_filename("Code_Reviewer"), "code-reviewer");
        assert_eq!(sanitize_filename("Test!@#Agent"), "testagent");
    }

    #[test]
    fn test_create_and_read_agent() {
        let dir = test_dir();
        // Temporarily override agents_dir by creating the directory
        // and then using list_agents after setting up the path.
        // Since agents_dir() uses home dir, we test parse/write directly.
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test-agent.md");

        let agent = CustomAgent {
            name: "test-agent".to_string(),
            description: "A test agent".to_string(),
            mode: AgentMode::Subagent,
            model: Some("coderouter/fast-model".to_string()),
            temperature: Some(0.3),
            prompt: "You are a test agent.".to_string(),
            permission: Some(AgentPermissions {
                edit: Some(PermissionLevel::Deny),
                bash: Some(BashPermission::Simple(PermissionLevel::Deny)),
                webfetch: None,
                task: None,
            }),
            ..Default::default()
        };

        write_agent_file(&path, &agent).unwrap();
        let parsed = parse_agent_file(&path).unwrap();

        assert_eq!(parsed.name, "test-agent");
        assert_eq!(parsed.description, "A test agent");
        assert_eq!(parsed.mode, AgentMode::Subagent);
        assert_eq!(parsed.model, Some("fast-model".to_string()));
        assert_eq!(parsed.temperature, Some(0.3));
        assert_eq!(parsed.prompt, "You are a test agent.");
        assert!(parsed.permission.is_some());
        let perm = parsed.permission.unwrap();
        assert_eq!(perm.edit, Some(PermissionLevel::Deny));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_get_templates_returns_agents() {
        let templates = get_templates();
        assert!(!templates.is_empty());
        assert!(templates.iter().any(|t| t.id == "gas-optimizer"));
        assert!(templates.iter().any(|t| t.id == "code-reviewer"));
    }
}
