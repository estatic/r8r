import type { NodeInstance } from '../types/domain'

export interface SetupProblem {
  nodeId: string
  missing: string[]
}

const REQUIRED: { key: string; label: string }[] = [
  { key: 'model', label: 'Model' },
  { key: 'user_message', label: 'User message' },
]

function filled(value: unknown): boolean {
  return typeof value === 'string' && value.trim() !== ''
}

/**
 * AI Agent nodes that can't run because a setting only the node can hold is
 * empty. (The provider isn't listed: the backend infers it from the
 * credential type.) Disabled nodes never run, so they're skipped.
 */
export function agentSetupProblems(nodes: NodeInstance[]): SetupProblem[] {
  return nodes
    .filter((n) => n.node_type === 'ai.agent' && !n.disabled)
    .map((n) => ({ nodeId: n.id, missing: REQUIRED.filter((r) => !filled(n.parameters?.[r.key])).map((r) => r.label) }))
    .filter((p) => p.missing.length > 0)
}

export function describeSetupProblems(problems: SetupProblem[]): string {
  const parts = problems.map((p) => `AI Agent "${p.nodeId}" needs ${p.missing.join(' and ')}.`)
  return `${parts.join(' ')} Open the node to set them.`
}
