import type { ApplicationIdentity, AwakeAgentId, AwakeRule } from '../models/types';

export const AWAKE_AGENT_OPTIONS: ReadonlyArray<{
  id: AwakeAgentId;
  label: string;
  hint: string;
}> = [
  { id: 'codex', label: 'Codex', hint: 'Codex CLI activity' },
  { id: 'claude', label: 'Claude Code', hint: 'Claude Code activity' },
  { id: 'antigravity', label: 'Antigravity', hint: 'agy / antigravity activity' },
  { id: 'opencode', label: 'OpenCode / OMP', hint: 'opencode / omp activity' },
];

const agentLabels = new Map<AwakeAgentId, string>(
  AWAKE_AGENT_OPTIONS.map((agent) => [agent.id, agent.label])
);

export function awakeAgentLabel(agentId: AwakeAgentId): string {
  return agentLabels.get(agentId) ?? agentId;
}

export function selectedAwakeAgents(rule: AwakeRule): AwakeAgentId[] {
  return [...(rule.agent_ids ?? [])];
}

export function toggleAwakeAgent(
  selected: readonly AwakeAgentId[],
  agentId: AwakeAgentId
): AwakeAgentId[] {
  return selected.includes(agentId)
    ? selected.filter((candidate) => candidate !== agentId)
    : [...selected, agentId];
}

function applicationLabel(application: ApplicationIdentity | null | undefined): string {
  return application?.display_name?.trim() || 'this application';
}

function agentClause(agentIds: readonly AwakeAgentId[]): string {
  if (agentIds.length === 0) return '';
  const labels = agentIds.map(awakeAgentLabel);
  const joined = labels.length === 1
    ? labels[0]
    : labels.length === 2
      ? `${labels[0]} or ${labels[1]}`
      : `${labels.slice(0, -1).join(', ')}, or ${labels.at(-1)}`;
  return ` and any of ${joined} is active`;
}

function powerClause(powerCondition: AwakeRule['power_condition']): string {
  return powerCondition === 'ac_power_only'
    ? 'only while plugged in'
    : 'on AC or battery power';
}

function behaviorClause(behavior: AwakeRule['behavior']): string {
  return behavior === 'keep_display_awake'
    ? 'The display stays awake.'
    : 'The display may sleep.';
}

/**
 * Creates the user-facing contract for typed rules. It deliberately ignores
 * raw legacy matcher fields so the basic flow never leaks pipe syntax or
 * command-line matching details.
 */
export function awakeRuleSummary(rule: AwakeRule): string {
  if (!rule.application) {
    return `Custom process rule: ${rule.app_name || 'unnamed rule'}.`;
  }
  return `Keep this computer awake when ${applicationLabel(rule.application)} is open${agentClause(
    selectedAwakeAgents(rule)
  )}, ${powerClause(rule.power_condition)}. ${behaviorClause(rule.behavior)}`;
}

export function typedAwakeRule(rule: AwakeRule): boolean {
  return Boolean(rule.application);
}
