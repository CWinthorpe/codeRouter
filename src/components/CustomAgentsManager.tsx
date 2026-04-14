import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  Plus,
  Trash2,
  Edit2,
  Sparkles,
  Loader2,
  X,
  ChevronDown,
  ChevronUp,
  FileText,
  AlertTriangle,
  CheckCircle2,
  Eye,
  Search,
  Shield,
  Bug,
  CheckSquare,
  Zap,
} from 'lucide-react';
import { useStore } from '../store';
import { Toast } from '../components/Toast';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '@/components/ui/select';
import {
  listCustomAgents,
  createCustomAgent,
  updateCustomAgent,
  deleteCustomAgent,
  getAgentTemplates,
  enhanceAgentText,
  type AgentEnhanceRequest,
} from '../lib/ipc';
import type { CustomAgent, AgentTemplate, AgentMode, AgentPermissions, PermissionLevel, BashPermission } from '../types';

type LucideIcon = typeof FileText;

const ICON_MAP: Record<string, LucideIcon> = {
  Search,
  FileText,
  Shield,
  Bug,
  CheckSquare,
  Zap,
};

const YAML_SPECIAL_CHARS = /[:#{}\[\],&*?|<>=!%@\\-]/;
const YAML_RESERVED = new Set(['true', 'false', 'null', 'yes', 'no', 'on', 'off', '~']);

function yamlQuote(s: string): string {
  if (s === '') return '""';
  if (
    YAML_SPECIAL_CHARS.test(s) ||
    s.includes('\n') ||
    s.includes('"') ||
    s.includes("'") ||
    YAML_RESERVED.has(s.toLowerCase()) ||
    (Number.isFinite(Number(s)) && String(Number(s)) === s) ||
    s !== s.trim()
  ) {
    return '"' + s.replace(/\\/g, '\\\\').replace(/\n/g, '\\n').replace(/"/g, '\\"') + '"';
  }
  return s;
}

function cleanPermissions(perms: AgentPermissions): AgentPermissions | undefined {
  const clean: AgentPermissions = {};
  if (perms.edit) clean.edit = perms.edit;
  if (typeof perms.bash === 'string' || (typeof perms.bash === 'object' && perms.bash !== null && Object.keys(perms.bash).length > 0)) {
    clean.bash = perms.bash;
  }
  if (perms.webfetch) clean.webfetch = perms.webfetch;
  if (perms.task && Object.keys(perms.task).length > 0) clean.task = perms.task;
  return Object.keys(clean).length > 0 ? clean : undefined;
}

function isValidPermission(v: unknown): v is PermissionLevel {
  return typeof v === 'string' && (v === 'allow' || v === 'deny' || v === 'ask');
}

function isValidBashPermission(v: unknown): v is BashPermission {
  if (isValidPermission(v)) return true;
  if (typeof v === 'object' && v !== null && !Array.isArray(v)) {
    return Object.values(v as Record<string, unknown>).every(isValidPermission);
  }
  return false;
}

function emptyAgent(): CustomAgent {
  return {
    name: '',
    description: '',
    mode: 'subagent',
    model: undefined,
    prompt: '',
    temperature: undefined,
    steps: undefined,
    disable: undefined,
    hidden: undefined,
    color: undefined,
    topP: undefined,
    permissions: undefined,
  };
}

function agentFromTemplate(template: AgentTemplate, name: string): CustomAgent {
  return {
    name,
    description: template.agent.description,
    mode: template.agent.mode,
    model: undefined,
    prompt: template.agent.prompt,
    temperature: template.agent.temperature,
    steps: template.agent.steps,
    disable: undefined,
    hidden: template.agent.hidden,
    color: template.agent.color,
    topP: template.agent.topP,
    permissions: template.agent.permissions,
  };
}

function generateMarkdownPreview(agent: CustomAgent): string {
  const frontmatter: Record<string, unknown> = {
    description: agent.description,
    mode: agent.mode,
  };
  if (agent.model) frontmatter.model = agent.model.startsWith('coderouter/') ? agent.model : `coderouter/${agent.model}`;
  if (agent.temperature !== undefined) frontmatter.temperature = agent.temperature;
  if (agent.steps !== undefined) frontmatter.steps = agent.steps;
  if (agent.disable) frontmatter.disable = true;
  if (agent.hidden) frontmatter.hidden = true;
  if (agent.color) frontmatter.color = agent.color;
  if (agent.topP !== undefined) frontmatter.top_p = agent.topP;
  if (agent.permissions) {
    const perm: Record<string, unknown> = {};
    if (agent.permissions.edit) perm.edit = agent.permissions.edit;
    if (agent.permissions.bash) perm.bash = agent.permissions.bash;
    if (agent.permissions.webfetch) perm.webfetch = agent.permissions.webfetch;
    if (agent.permissions.task) perm.task = agent.permissions.task;
    if (Object.keys(perm).length > 0) frontmatter.permission = perm;
  }
  if (agent.additional) {
    for (const [key, value] of Object.entries(agent.additional)) {
      frontmatter[key] = value;
    }
  }

  const yamlLines = Object.entries(frontmatter).map(([key, value]) => {
    if (typeof value === 'object' && value !== null) {
      return `${yamlQuote(key)}:\n  ${yamlifyObject(value as Record<string, unknown>, '  ')}`;
    }
    return `${yamlQuote(key)}: ${typeof value === 'string' ? yamlQuote(value) : value}`;
  });

  return `---\n${yamlLines.join('\n')}\n---\n\n${agent.prompt}`;
}

function yamlifyObject(obj: Record<string, unknown>, indent: string): string {
  return Object.entries(obj)
    .map(([k, v]) => {
      if (typeof v === 'object' && v !== null && !Array.isArray(v)) {
        return `${yamlQuote(k)}:\n${indent}  ${yamlifyObject(v as Record<string, unknown>, indent + '  ')}`;
      }
      return `${yamlQuote(k)}: ${typeof v === 'string' ? yamlQuote(v) : v}`;
    })
    .join(`\n${indent}`);
}

const PERMISSION_OPTIONS: { value: PermissionLevel; label: string }[] = [
  { value: 'allow', label: 'Allow' },
  { value: 'deny', label: 'Deny' },
  { value: 'ask', label: 'Ask' },
];

function AiEnhanceButton({
  text,
  enhanceType,
  modelGroup,
  onEnhanced,
}: {
  text: string;
  enhanceType: 'description' | 'prompt' | 'suggestions';
  modelGroup: string | null;
  onEnhanced: (result: string) => void;
}) {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleEnhance = useCallback(async () => {
    if (!text.trim()) return;
    if (!modelGroup) {
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const request: AgentEnhanceRequest = {
        text,
        enhanceType,
        modelGroup,
      };
      const response = await enhanceAgentText(request);
      onEnhanced(response.result);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [text, enhanceType, modelGroup, onEnhanced]);

  return (
    <div className="flex items-center gap-2">
      <button
        onClick={handleEnhance}
        disabled={loading || !text.trim() || !modelGroup}
        className="flex items-center gap-1.5 rounded-md bg-violet-600/20 px-2.5 py-1.5 text-xs font-medium text-violet-300 transition-colors hover:bg-violet-600/30 disabled:opacity-40"
        title={enhanceType === 'suggestions' ? 'Get AI suggestions for settings' : `AI enhance ${enhanceType}`}
      >
        {loading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Sparkles className="h-3.5 w-3.5" />}
        {enhanceType === 'suggestions' ? 'AI Suggest' : 'AI Enhance'}
      </button>
      {error && <span className="text-xs text-red-400">{error}</span>}
    </div>
  );
}

function CustomAgentForm({
  agent,
  agents,
  onChange,
  groupAliases,
  onCancel,
  onSave,
  isEditing,
  saving,
  onError,
}: {
  agent: CustomAgent;
  agents: CustomAgent[];
  onChange: (agent: CustomAgent) => void;
  groupAliases: string[];
  onCancel: () => void;
  onSave: () => void;
  isEditing: boolean;
  saving: boolean;
  onError: (message: string) => void;
}) {
  const [showPreview, setShowPreview] = useState(false);
  const [previewMd, setPreviewMd] = useState('');
  const [nameError, setNameError] = useState<string | null>(null);
  const [newBashPattern, setNewBashPattern] = useState('');
  const [newBashLevel, setNewBashLevel] = useState<PermissionLevel>('allow');
  const [newTaskPattern, setNewTaskPattern] = useState('');
  const [newTaskLevel, setNewTaskLevel] = useState<PermissionLevel>('allow');

  const INVALID_NAME_RE = /[^a-zA-Z0-9-]/;

  const isDuplicateName = !isEditing && agent.name.length > 0 && agents.some((a) => a.name === agent.name);

  useEffect(() => {
    if (showPreview) {
      setPreviewMd(generateMarkdownPreview(agent));
    }
  }, [agent, showPreview]);

  const updateField = useCallback(<K extends keyof CustomAgent>(key: K, value: CustomAgent[K]) => {
    onChange({ ...agent, [key]: value });
  }, [agent, onChange]);

  return (
    <div className="rounded-lg border border-zinc-700 bg-zinc-900 p-5">
      <div className="mb-4 flex items-center justify-between">
        <h3 className="text-base font-semibold text-zinc-100">
          {isEditing ? 'Edit Agent' : 'New Agent'}
        </h3>
        <button onClick={onCancel} className="rounded-md p-1 text-zinc-400 hover:bg-zinc-800 hover:text-zinc-200" aria-label="Cancel editing">
          <X className="h-4 w-4" />
        </button>
      </div>

      <div className="flex flex-col gap-4">
        <div className="flex flex-col gap-1.5">
          <label htmlFor="agent-name" className="text-sm font-medium text-zinc-300">Agent Name</label>
          <input
            id="agent-name"
            type="text"
            value={agent.name}
            onChange={(e) => {
              const val = e.target.value;
              updateField('name', val);
              setNameError(INVALID_NAME_RE.test(val) ? 'Only letters, numbers, and hyphens allowed' : null);
            }}
            placeholder="e.g., code-reviewer"
            disabled={isEditing}
            className="rounded-md border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-100 placeholder-zinc-500 focus:border-emerald-500 focus:outline-none focus:ring-1 focus:ring-emerald-500 disabled:opacity-50"
          />
          {nameError && <span className="text-xs text-red-400">{nameError}</span>}
          {isDuplicateName && <span className="text-xs text-red-400">An agent with this name already exists</span>}
        </div>

        <div className="flex flex-col gap-1.5">
          <div className="flex items-center justify-between">
            <label htmlFor="agent-description" className="text-sm font-medium text-zinc-300">Description</label>
            <AiEnhanceButton
              text={agent.description}
              enhanceType="description"
              modelGroup={agent.model ?? null}
              onEnhanced={(result) => updateField('description', result)}
            />
          </div>
          <input
            id="agent-description"
            type="text"
            value={agent.description}
            onChange={(e) => updateField('description', e.target.value)}
            placeholder="What does this agent do and when to use it?"
            className="rounded-md border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-100 placeholder-zinc-500 focus:border-emerald-500 focus:outline-none focus:ring-1 focus:ring-emerald-500"
          />
        </div>

        <div className="flex flex-col gap-1.5">
          <label htmlFor="agent-mode" className="text-sm font-medium text-zinc-300">Mode</label>
          <Select value={agent.mode} onValueChange={(v) => updateField('mode', v as AgentMode)}>
            <SelectTrigger id="agent-mode" className="border-zinc-700 bg-zinc-800 text-zinc-100">
              <SelectValue />
            </SelectTrigger>
            <SelectContent className="bg-zinc-800 border-zinc-700">
              <SelectItem value="primary" className="text-zinc-100 focus:bg-zinc-700">Primary</SelectItem>
              <SelectItem value="subagent" className="text-zinc-100 focus:bg-zinc-700">Subagent</SelectItem>
              <SelectItem value="all" className="text-zinc-100 focus:bg-zinc-700">All</SelectItem>
            </SelectContent>
          </Select>
        </div>

        <div className="flex flex-col gap-1.5">
          <label htmlFor="agent-model" className="text-sm font-medium text-zinc-300">Model Group</label>
          {/* __none__ sentinel: known limitation — if a model group alias is literally "__none__", it cannot be selected. This is extremely unlikely. */}
          <Select value={agent.model || '__none__'} onValueChange={(v) => updateField('model', v === '__none__' ? undefined : v)}>
            <SelectTrigger id="agent-model" className="border-zinc-700 bg-zinc-800 text-zinc-100">
              <SelectValue placeholder="Select a model group" />
            </SelectTrigger>
            <SelectContent className="bg-zinc-800 border-zinc-700">
              <SelectItem value="__none__" className="text-zinc-100 focus:bg-zinc-700">— none (uses default) —</SelectItem>
              {groupAliases.map((alias) => (
                <SelectItem key={alias} value={alias} className="text-zinc-100 focus:bg-zinc-700">
                  {alias}
                </SelectItem>
              ))}
              {agent.model && !groupAliases.includes(agent.model) && (
                <SelectItem value={agent.model} className="text-zinc-100 focus:bg-zinc-700">
                  {agent.model} (deleted group)
                </SelectItem>
              )}
            </SelectContent>
          </Select>
        </div>

        <div className="flex flex-col gap-1.5">
          <div className="flex items-center justify-between">
            <label htmlFor="agent-temperature" className="text-sm font-medium text-zinc-300">
              Temperature
              {agent.temperature !== undefined && (
                <span className="ml-2 text-xs text-zinc-500">
                  ({agent.temperature <= 0.2 ? 'focused' : agent.temperature <= 0.5 ? 'balanced' : 'creative'})
                </span>
              )}
            </label>
            <div className="flex items-center gap-2">
              <input
                id="agent-temperature"
                type="range"
                min="0"
                max="1"
                step="0.1"
                value={agent.temperature ?? 0.3}
                onChange={(e) => updateField('temperature', parseFloat(e.target.value))}
                aria-valuetext={agent.temperature?.toFixed(1) ?? '0.3'}
                className="w-24 accent-emerald-500"
              />
              <span className="w-8 text-right text-xs text-zinc-400">
                {agent.temperature?.toFixed(1) ?? '0.3'}
              </span>
              <button
                onClick={() => updateField('temperature', undefined)}
                className="text-xs text-zinc-500 hover:text-zinc-300"
              >
                Reset
              </button>
            </div>
          </div>
        </div>

        <div className="flex flex-col gap-1.5">
          <div className="flex items-center justify-between">
            <label htmlFor="agent-prompt" className="text-sm font-medium text-zinc-300">System Prompt</label>
            <AiEnhanceButton
              text={agent.prompt}
              enhanceType="prompt"
              modelGroup={agent.model ?? null}
              onEnhanced={(result) => updateField('prompt', result)}
            />
          </div>
          <textarea
            id="agent-prompt"
            value={agent.prompt}
            onChange={(e) => updateField('prompt', e.target.value)}
            placeholder="You are a specialized agent for..."
            rows={8}
            className="rounded-md border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-100 placeholder-zinc-500 focus:border-emerald-500 focus:outline-none focus:ring-1 focus:ring-emerald-500 font-mono leading-relaxed"
          />
        </div>

        <div className="flex flex-col gap-1.5">
          <div className="flex items-center justify-between">
            <span className="text-sm font-medium text-zinc-300">Permissions</span>
            <AiEnhanceButton
              text={`${agent.description}\n\n${agent.prompt}`}
              enhanceType="suggestions"
              modelGroup={agent.model ?? null}
              onEnhanced={(result) => {
                try {
                  const suggestions = JSON.parse(result);
                  const perms: AgentPermissions = { ...(agent.permissions ?? {}) };
                  const updates: Partial<Pick<CustomAgent, 'temperature' | 'permissions'>> = {};
                  if (isValidPermission(suggestions.edit_permission)) perms.edit = suggestions.edit_permission;
                  if (isValidBashPermission(suggestions.bash_permission)) perms.bash = suggestions.bash_permission;
                  if (isValidPermission(suggestions.webfetch_permission)) perms.webfetch = suggestions.webfetch_permission;
                  if (
                    typeof suggestions.temperature === 'number' &&
                    suggestions.temperature >= 0 &&
                    suggestions.temperature <= 1
                  ) {
                    updates.temperature = suggestions.temperature;
                  }
                  updates.permissions = cleanPermissions(perms);
                  onChange({ ...agent, ...updates });
                } catch {
                  onError('Failed to parse AI suggestions as JSON');
                }
              }}
            />
          </div>
          <div className="flex flex-wrap gap-4 rounded-md border border-zinc-700 bg-zinc-800/50 p-3">
            {(['edit', 'bash', 'webfetch'] as const).map((tool) => {
              const currentValue = agent.permissions?.[tool];
              const isBashObject = tool === 'bash' && typeof currentValue === 'object' && currentValue !== null && !Array.isArray(currentValue);
              return (
                <div key={tool} className="flex items-center gap-2">
                  <span className="text-xs font-medium text-zinc-400 capitalize">{tool}</span>
                  {isBashObject ? (
                    <div className="flex items-center gap-1.5">
                      <span className="text-xs rounded bg-zinc-700 px-1.5 py-0.5 text-zinc-300">Custom rules</span>
                      <button
                        onClick={() => {
                          const perms: AgentPermissions = { ...(agent.permissions ?? {}) };
                          delete perms.bash;
                          onChange({ ...agent, permissions: cleanPermissions(perms) });
                        }}
                        className="text-xs text-zinc-500 hover:text-zinc-300"
                        title="Reset to simple mode"
                      >
                        Reset
                      </button>
                    </div>
                  ) : (
                    <Select
                      value={typeof currentValue === 'string' ? currentValue : '__unset__'}
                      onValueChange={(v) => {
                        const perms: AgentPermissions = { ...(agent.permissions ?? {}) };
                        if (v === '__unset__') {
                          delete perms[tool];
                        } else {
                          (perms as Record<string, unknown>)[tool] = v;
                        }
                        onChange({ ...agent, permissions: cleanPermissions(perms) });
                      }}
                    >
                      <SelectTrigger className="h-7 w-24 border-zinc-600 bg-zinc-800 text-xs text-zinc-100">
                        <SelectValue placeholder="—" />
                      </SelectTrigger>
                      <SelectContent className="bg-zinc-800 border-zinc-700">
                        <SelectItem value="__unset__" className="text-xs text-zinc-100 focus:bg-zinc-700">—</SelectItem>
                        {PERMISSION_OPTIONS.map((opt) => (
                          <SelectItem key={opt.value} value={opt.value} className="text-xs text-zinc-100 focus:bg-zinc-700">
                            {opt.label}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  )}
                </div>
              );
            })}
          </div>

          {typeof agent.permissions?.bash === 'object' && agent.permissions.bash !== null && (
            <div className="rounded-md border border-zinc-700 bg-zinc-800/50 p-3">
              <div className="mb-2 text-xs font-medium text-zinc-400">Bash Rules</div>
              {Object.entries(agent.permissions.bash as Record<string, PermissionLevel>).map(([pattern, level]) => (
                <div key={pattern} className="flex items-center gap-2 mb-1">
                  <code className="text-xs text-zinc-300 flex-1 rounded bg-zinc-700 px-1.5 py-0.5">{pattern}</code>
                  <span className="text-xs text-zinc-400 w-12">{level}</span>
                  <button
                    onClick={() => {
                      const rules = { ...(agent.permissions?.bash as Record<string, PermissionLevel>) };
                      delete rules[pattern];
                      const perms: AgentPermissions = { ...(agent.permissions ?? {}) };
                      perms.bash = Object.keys(rules).length > 0 ? rules : undefined;
                      onChange({ ...agent, permissions: cleanPermissions(perms) });
                    }}
                    className="rounded-md p-0.5 text-zinc-500 hover:text-red-400"
                    aria-label={`Remove bash rule ${pattern}`}
                  >
                    <X className="h-3 w-3" />
                  </button>
                </div>
              ))}
              <div className="flex items-center gap-2 mt-2">
                <input
                  id="new-bash-pattern"
                  type="text"
                  value={newBashPattern}
                  onChange={(e) => setNewBashPattern(e.target.value)}
                  placeholder="e.g., npm *"
                  className="flex-1 rounded border border-zinc-600 bg-zinc-800 px-2 py-1 text-xs text-zinc-100 placeholder-zinc-500 focus:border-emerald-500 focus:outline-none"
                />
                <select
                  id="new-bash-level"
                  value={newBashLevel}
                  onChange={(e) => setNewBashLevel(e.target.value as PermissionLevel)}
                  className="rounded border border-zinc-600 bg-zinc-800 px-2 py-1 text-xs text-zinc-100"
                >
                  {PERMISSION_OPTIONS.map((opt) => (
                    <option key={opt.value} value={opt.value}>{opt.label}</option>
                  ))}
                </select>
                <button
                  onClick={() => {
                    if (!newBashPattern.trim()) return;
                    const rules = { ...((agent.permissions?.bash as Record<string, PermissionLevel>) ?? {}) };
                    rules[newBashPattern.trim()] = newBashLevel;
                    const perms: AgentPermissions = { ...(agent.permissions ?? {}) };
                    perms.bash = rules;
                    onChange({ ...agent, permissions: cleanPermissions(perms) });
                    setNewBashPattern('');
                  }}
                  className="text-xs text-emerald-400 hover:text-emerald-300"
                >
                  Add
                </button>
              </div>
            </div>
          )}

          <div className="rounded-md border border-zinc-700 bg-zinc-800/50 p-3">
            <div className="mb-2 text-xs font-medium text-zinc-400">Task Permissions</div>
            {agent.permissions?.task && Object.keys(agent.permissions.task).length > 0 ? (
              Object.entries(agent.permissions.task).map(([pattern, level]) => (
                <div key={pattern} className="flex items-center gap-2 mb-1">
                  <code className="text-xs text-zinc-300 flex-1 rounded bg-zinc-700 px-1.5 py-0.5">{pattern}</code>
                  <span className="text-xs text-zinc-400 w-12">{level}</span>
                  <button
                    onClick={() => {
                      const tasks = { ...(agent.permissions?.task ?? {}) };
                      delete tasks[pattern];
                      const perms: AgentPermissions = { ...(agent.permissions ?? {}) };
                      perms.task = Object.keys(tasks).length > 0 ? tasks : undefined;
                      onChange({ ...agent, permissions: cleanPermissions(perms) });
                    }}
                    className="rounded-md p-0.5 text-zinc-500 hover:text-red-400"
                    aria-label={`Remove task rule ${pattern}`}
                  >
                    <X className="h-3 w-3" />
                  </button>
                </div>
              ))
            ) : (
              <span className="text-xs text-zinc-500">No task rules configured</span>
            )}
            <div className="flex items-center gap-2 mt-2">
              <input
                id="new-task-pattern"
                type="text"
                value={newTaskPattern}
                onChange={(e) => setNewTaskPattern(e.target.value)}
                placeholder="Agent pattern, e.g., reviewer"
                className="flex-1 rounded border border-zinc-600 bg-zinc-800 px-2 py-1 text-xs text-zinc-100 placeholder-zinc-500 focus:border-emerald-500 focus:outline-none"
              />
              <select
                id="new-task-level"
                value={newTaskLevel}
                onChange={(e) => setNewTaskLevel(e.target.value as PermissionLevel)}
                className="rounded border border-zinc-600 bg-zinc-800 px-2 py-1 text-xs text-zinc-100"
              >
                {PERMISSION_OPTIONS.map((opt) => (
                  <option key={opt.value} value={opt.value}>{opt.label}</option>
                ))}
              </select>
              <button
                onClick={() => {
                  if (!newTaskPattern.trim()) return;
                  const tasks = { ...(agent.permissions?.task ?? {}) };
                  tasks[newTaskPattern.trim()] = newTaskLevel;
                  const perms: AgentPermissions = { ...(agent.permissions ?? {}) };
                  perms.task = tasks;
                  onChange({ ...agent, permissions: cleanPermissions(perms) });
                  setNewTaskPattern('');
                }}
                className="text-xs text-emerald-400 hover:text-emerald-300"
              >
                Add
              </button>
            </div>
          </div>
        </div>

        <div className="flex flex-col gap-1.5">
          <span className="text-sm font-medium text-zinc-300">Advanced</span>
          <div className="flex flex-wrap gap-4 rounded-md border border-zinc-700 bg-zinc-800/50 p-3">
            <div className="flex items-center gap-2">
              <label htmlFor="agent-steps" className="text-xs text-zinc-400">Max Steps</label>
              <input
                id="agent-steps"
                type="number"
                min="1"
                value={agent.steps ?? ''}
                onChange={(e) => { const n = e.target.value ? parseInt(e.target.value, 10) : NaN; updateField('steps', Number.isNaN(n) || n < 1 ? undefined : n); }}
                placeholder="—"
                className="w-16 rounded border border-zinc-600 bg-zinc-800 px-2 py-1 text-xs text-zinc-100 placeholder-zinc-500 focus:border-emerald-500 focus:outline-none"
              />
            </div>

            <div className="flex items-center gap-2">
              <label htmlFor="agent-top-p" className="text-xs text-zinc-400">Top P</label>
              <input
                id="agent-top-p"
                type="number"
                min="0"
                max="1"
                step="0.1"
                value={agent.topP ?? ''}
                onChange={(e) => { const n = e.target.value ? parseFloat(e.target.value) : NaN; updateField('topP', Number.isNaN(n) || n < 0 || n > 1 ? undefined : n); }}
                placeholder="—"
                className="w-16 rounded border border-zinc-600 bg-zinc-800 px-2 py-1 text-xs text-zinc-100 placeholder-zinc-500 focus:border-emerald-500 focus:outline-none"
              />
            </div>

            <div className="flex items-center gap-2">
              <label htmlFor="agent-color" className="text-xs text-zinc-400">Color</label>
              <input
                id="agent-color"
                type="text"
                value={agent.color ?? ''}
                onChange={(e) => updateField('color', e.target.value || undefined)}
                placeholder="#hex or theme"
                className="w-28 rounded border border-zinc-600 bg-zinc-800 px-2 py-1 text-xs text-zinc-100 placeholder-zinc-500 focus:border-emerald-500 focus:outline-none"
              />
            </div>

            {/* Checkbox `|| undefined` is intentional: absent = false on both read and write sides */}
            <label htmlFor="agent-hidden" className="flex items-center gap-2 text-xs text-zinc-400">
              <input
                id="agent-hidden"
                type="checkbox"
                checked={agent.hidden ?? false}
                onChange={(e) => updateField('hidden', e.target.checked || undefined)}
                className="rounded border-zinc-600 bg-zinc-800 text-emerald-500 focus:ring-emerald-500"
              />
              Hidden
            </label>

            <label htmlFor="agent-disable" className="flex items-center gap-2 text-xs text-zinc-400">
              <input
                id="agent-disable"
                type="checkbox"
                checked={agent.disable ?? false}
                onChange={(e) => updateField('disable', e.target.checked || undefined)}
                className="rounded border-zinc-600 bg-zinc-800 text-emerald-500 focus:ring-emerald-500"
              />
              Disabled
            </label>
          </div>
        </div>

        <div>
          <button
            onClick={() => setShowPreview(!showPreview)}
            className="flex items-center gap-2 text-sm font-medium text-zinc-300 transition-colors hover:text-zinc-100"
          >
            <Eye className="h-4 w-4" />
            {showPreview ? 'Hide' : 'Show'} Markdown Preview
            {showPreview ? <ChevronUp className="h-4 w-4" /> : <ChevronDown className="h-4 w-4" />}
          </button>
          {showPreview && (
            <div className="mt-2 relative rounded-md border border-zinc-800 bg-zinc-950">
              <div className="absolute right-3 top-3 flex items-center gap-1.5 text-xs text-zinc-500">
                <FileText className="h-3.5 w-3.5" />
                Markdown
              </div>
              <pre className="overflow-auto p-4 pr-16 text-xs leading-relaxed text-zinc-300 font-mono whitespace-pre-wrap">
                {previewMd}
              </pre>
            </div>
          )}
        </div>

        <div className="flex items-center justify-end gap-3 pt-2">
          <button
            onClick={onCancel}
            className="rounded-md bg-zinc-800 px-4 py-2 text-sm font-medium text-zinc-300 transition-colors hover:bg-zinc-700"
          >
            Cancel
          </button>
          <button
            onClick={onSave}
            disabled={!agent.name.trim() || !agent.description.trim() || !agent.prompt.trim() || nameError !== null || isDuplicateName || saving}
            className="flex items-center gap-2 rounded-md bg-emerald-600 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-emerald-500 disabled:opacity-50"
          >
            {saving ? <Loader2 className="h-4 w-4 animate-spin" /> : <CheckCircle2 className="h-4 w-4" />}
            {saving ? 'Saving…' : isEditing ? 'Update Agent' : 'Create Agent'}
          </button>
        </div>
      </div>
    </div>
  );
}

export default function CustomAgentsManager() {
  const groups = useStore((s) => s.groups);
  const groupAliases = useMemo(() => groups.map((g) => g.alias), [groups]);

  const [agents, setAgents] = useState<CustomAgent[]>([]);
  const [templates, setTemplates] = useState<AgentTemplate[]>([]);
  const [loading, setLoading] = useState(true);
  const [showForm, setShowForm] = useState(false);
  const [editingAgent, setEditingAgent] = useState<CustomAgent | null>(null);
  const [formAgent, setFormAgent] = useState<CustomAgent>(emptyAgent());
  const [showTemplates, setShowTemplates] = useState(false);
  const [toasts, setToasts] = useState<{ id: number; type: 'success' | 'error'; message: string }[]>([]);
  const [deleting, setDeleting] = useState<Set<string>>(new Set());
  const [saving, setSaving] = useState(false);
  const toastCounterRef = useRef(0);
  const toastTimeoutsRef = useRef<Set<ReturnType<typeof setTimeout>>>(new Set());

  useEffect(() => {
    return () => {
      toastTimeoutsRef.current.forEach((tid) => clearTimeout(tid));
    };
  }, []);

  const addToast = useCallback((type: 'success' | 'error', message: string) => {
    const id = Date.now() * 1000 + (++toastCounterRef.current);
    setToasts((prev) => [...prev, { id, type, message }]);
    const tid = setTimeout(() => {
      setToasts((prev) => prev.filter((t) => t.id !== id));
      toastTimeoutsRef.current.delete(tid);
    }, 4000);
    toastTimeoutsRef.current.add(tid);
  }, []);

  const loadData = useCallback(async () => {
    setLoading(true);
    try {
      const [agentsData, templatesData] = await Promise.all([
        listCustomAgents(),
        getAgentTemplates(),
      ]);
      setAgents(agentsData);
      setTemplates(templatesData);
    } catch {
      // IPC may fail outside Tauri
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadData();
  }, [loadData]);

  const handleCreate = useCallback(() => {
    setEditingAgent(null);
    setFormAgent(emptyAgent());
    setShowForm(true);
    setShowTemplates(false);
  }, []);

  const handleEdit = useCallback((agent: CustomAgent) => {
    setEditingAgent(agent);
    setFormAgent({ ...agent });
    setShowForm(true);
    setShowTemplates(false);
  }, []);

  const handleDelete = useCallback(async (name: string) => {
    setDeleting((prev) => new Set(prev).add(name));
    try {
      await deleteCustomAgent(name);
      setAgents((prev) => prev.filter((a) => a.name !== name));
      addToast('success', `Agent "${name}" deleted`);
    } catch (e: unknown) {
      addToast('error', e instanceof Error ? e.message : String(e));
    } finally {
      setDeleting((prev) => {
        const next = new Set(prev);
        next.delete(name);
        return next;
      });
    }
  }, [addToast]);

  const handleSave = useCallback(async () => {
    setSaving(true);
    try {
      if (editingAgent) {
        const updated = await updateCustomAgent(editingAgent.name, formAgent);
        setAgents((prev) => prev.map((a) => (a.name === editingAgent.name ? updated : a)));
        addToast('success', `Agent "${formAgent.name}" updated`);
      } else {
        const created = await createCustomAgent(formAgent);
        setAgents((prev) => [...prev, created]);
        addToast('success', `Agent "${formAgent.name}" created`);
      }
      setShowForm(false);
      setEditingAgent(null);
    } catch (e: unknown) {
      addToast('error', e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }, [editingAgent, formAgent, addToast]);

  const handleCancelForm = useCallback(() => {
    setShowForm(false);
    setEditingAgent(null);
  }, []);

  const handleTemplateSelect = useCallback((template: AgentTemplate) => {
    const name = template.id;
    setEditingAgent(null);
    setFormAgent(agentFromTemplate(template, name));
    setShowForm(true);
    setShowTemplates(false);
  }, []);

  if (loading) {
    return (
      <div className="flex items-center gap-2 rounded-md bg-zinc-800/50 px-4 py-6 text-sm text-zinc-400">
        <Loader2 className="h-4 w-4 animate-spin" />
        Loading custom agents…
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-base font-semibold text-zinc-100">Custom Agents & Subagents</h3>
          <p className="mt-0.5 text-sm text-zinc-400">
            Create specialized agents using templates or from scratch. Agents are saved as markdown files.
          </p>
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={() => setShowTemplates(!showTemplates)}
            className="flex items-center gap-1.5 rounded-md bg-zinc-800 px-3 py-2 text-sm font-medium text-zinc-300 transition-colors hover:bg-zinc-700"
          >
            <FileText className="h-4 w-4" />
            From Template
          </button>
          <button
            onClick={handleCreate}
            className="flex items-center gap-1.5 rounded-md bg-emerald-600 px-3 py-2 text-sm font-medium text-white transition-colors hover:bg-emerald-500"
          >
            <Plus className="h-4 w-4" />
            New Agent
          </button>
        </div>
      </div>

      {showTemplates && (
        <div className="rounded-lg border border-zinc-700 bg-zinc-900 p-5">
          <div className="mb-3 flex items-center justify-between">
            <h4 className="text-sm font-semibold text-zinc-100">Choose a Template</h4>
            <button onClick={() => setShowTemplates(false)} className="rounded-md p-1 text-zinc-400 hover:bg-zinc-800" aria-label="Close templates">
              <X className="h-4 w-4" />
            </button>
          </div>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {templates.map((template) => {
              const IconComponent = ICON_MAP[template.icon];
              return (
                <button
                  key={template.id}
                  onClick={() => handleTemplateSelect(template)}
                  className="flex flex-col rounded-lg border border-zinc-700 bg-zinc-800/50 p-4 text-left transition-colors hover:border-emerald-600/50 hover:bg-zinc-800"
                >
                  <div className="mb-2 flex items-center gap-2">
                    {IconComponent ? <IconComponent className="h-5 w-5 text-zinc-300" /> : <span className="text-lg">{template.icon}</span>}
                    <span className="text-sm font-semibold text-zinc-100">{template.name}</span>
                  </div>
                  <p className="text-xs text-zinc-400">{template.description}</p>
                </button>
              );
            })}
          </div>
        </div>
      )}

      {agents.length === 0 && !showForm ? (
        <div className="flex flex-col items-center gap-3 rounded-lg border border-dashed border-zinc-700 py-12 text-center">
          <AlertTriangle className="h-8 w-8 text-zinc-600" />
          <p className="text-sm text-zinc-400">No custom agents yet. Create one or use a template to get started.</p>
        </div>
      ) : (
        <div className="flex flex-col gap-3">
          {agents.map((agent) => (
            <div
              key={agent.name}
              className="flex items-center justify-between rounded-lg border border-zinc-700 bg-zinc-900/60 p-4"
            >
              <div className="flex-1">
                <div className="flex items-center gap-2">
                  <span className="text-sm font-semibold text-zinc-100">{agent.name}</span>
                  <span
                    className={`rounded-full px-2 py-0.5 text-xs font-medium ${
                      agent.mode === 'primary'
                        ? 'bg-blue-600/20 text-blue-300'
                        : agent.mode === 'subagent'
                        ? 'bg-purple-600/20 text-purple-300'
                        : 'bg-zinc-600/20 text-zinc-300'
                    }`}
                  >
                    {agent.mode}
                  </span>
                  {agent.disable && (
                    <span className="rounded-full bg-red-600/20 px-2 py-0.5 text-xs font-medium text-red-300">
                      disabled
                    </span>
                  )}
                </div>
                <p className="mt-1 text-xs text-zinc-400">{agent.description}</p>
                {agent.model && (
                  <p className="mt-1 text-xs text-zinc-500">
                    Model: <code className="rounded bg-zinc-800 px-1">coderouter/{agent.model}</code>
                  </p>
                )}
              </div>
              <div className="flex items-center gap-2">
                <button
                  onClick={() => handleEdit(agent)}
                  className="rounded-md p-1.5 text-zinc-400 transition-colors hover:bg-zinc-800 hover:text-zinc-200"
                  title="Edit agent"
                  aria-label={`Edit agent ${agent.name}`}
                >
                  <Edit2 className="h-4 w-4" />
                </button>
                <button
                  onClick={() => { if (window.confirm(`Delete agent "${agent.name}"?`)) handleDelete(agent.name); }}
                  disabled={deleting.has(agent.name)}
                  className="rounded-md p-1.5 text-zinc-400 transition-colors hover:bg-red-900/50 hover:text-red-300 disabled:opacity-50"
                  title="Delete agent"
                  aria-label={`Delete agent ${agent.name}`}
                >
                  {deleting.has(agent.name) ? (
                    <Loader2 className="h-4 w-4 animate-spin" />
                  ) : (
                    <Trash2 className="h-4 w-4" />
                  )}
                </button>
              </div>
            </div>
          ))}
        </div>
      )}

      {showForm && (
        <CustomAgentForm
          agent={formAgent}
          agents={agents}
          onChange={setFormAgent}
          groupAliases={groupAliases}
          onCancel={handleCancelForm}
          onSave={handleSave}
          isEditing={!!editingAgent}
          saving={saving}
          onError={(message) => addToast('error', message)}
        />
      )}

      <div className="fixed bottom-4 right-4 z-50 flex flex-col gap-2">
        {toasts.map((toast) => (
          <Toast key={toast.id} type={toast.type} message={toast.message} />
        ))}
      </div>
    </div>
  );
}
