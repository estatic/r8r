import type { Connection, Execution, NodeCategory } from '../types/domain'

/** How the canvas colours a node or a link for the run on screen. */
export type RunState = 'success' | 'running' | 'error' | 'pending'

/** null when no run is shown: the canvas then looks as usual. */
export function nodeRunState(execution: Execution | null, nodeId: string): RunState | null {
  if (!execution) return null
  const run = execution.node_runs?.[nodeId]
  if (!run || run.status === 'skipped') return 'pending'
  return run.status
}

function handleOf(c: Connection): string {
  return c.error ? 'error' : String(c.from_output)
}

function sentCount(execution: Execution, c: Connection): number {
  return execution.node_runs?.[c.from_node]?.counts[handleOf(c)] ?? 0
}

/** A link is coloured by what went through it. */
export function edgeRunState(execution: Execution | null, c: Connection): RunState | null {
  if (!execution) return null
  if (sentCount(execution, c) === 0) return 'pending'
  if (c.error) return 'error'
  return execution.node_runs?.[c.to_node]?.status === 'running' ? 'running' : 'success'
}

/** "N items" above a link, as n8n shows; nothing when none went through. */
export function itemsLabel(execution: Execution | null, c: Connection): string | undefined {
  if (!execution) return undefined
  const n = sentCount(execution, c)
  if (n === 0) return undefined
  return n === 1 ? '1 item' : `${n} items`
}

export type NodeShape = 'start' | 'end' | 'box'

/** Triggers start a workflow; a node nothing follows ends it. */
export function nodeShape(nodeId: string, category: NodeCategory | undefined, connections: Connection[]): NodeShape {
  if (category === 'trigger') return 'start'
  return connections.some((c) => c.from_node === nodeId) ? 'box' : 'end'
}

export const RUN_COLORS: Record<RunState, string> = {
  success: '#16a34a',
  running: '#eab308',
  error: '#dc2626',
  pending: '#9ca3af',
}
