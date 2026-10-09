/** What the node view shows for a node in a run: its input, its outputs, its error. */
import type { Execution, Item, Workflow } from '../types/domain'

export interface PortData {
  label: string
  items: Item[]
}

/** The items the node received: as recorded, else what its inputs were sent (older runs). */
export function nodeInput(execution: Execution | null, workflow: Workflow, nodeId: string): Item[] | null {
  if (!execution) return null
  const run = execution.node_runs?.[nodeId]
  if (run?.input && run.input.length > 0) return run.input
  const into = workflow.connections.filter((c) => c.to_node === nodeId)
  if (into.length === 0) return run ? [] : null
  const items: Item[] = []
  let any = false
  for (const c of into) {
    const from = execution.node_runs?.[c.from_node]
    if (!from && !execution.node_outputs[c.from_node]) continue
    any = true
    if (c.error) items.push(...(from?.error_items ?? []))
    else items.push(...(from?.outputs?.[c.from_output] ?? (c.from_output === 0 ? (execution.node_outputs[c.from_node] ?? []) : [])))
  }
  return any ? items : null
}

/** The node's outputs, one per port (labels from its type), plus its error output when it sent any. */
export function nodeOutputs(execution: Execution | null, nodeId: string, portNames: string[]): PortData[] | null {
  if (!execution) return null
  const run = execution.node_runs?.[nodeId]
  if (!run && !execution.node_outputs[nodeId]) return null
  if (run?.status === 'error' && !(run.outputs ?? []).some((p) => p.length > 0)) return []
  const ports = run?.outputs && run.outputs.length > 0 ? run.outputs : [execution.node_outputs[nodeId] ?? []]
  const out = ports.map((items, i) => ({ label: portNames[i] && portNames[i] !== 'main' ? portNames[i] : ports.length > 1 ? `Output ${i}` : 'Output', items }))
  if (run?.error_items && run.error_items.length > 0) out.push({ label: 'error', items: run.error_items })
  return out
}

/** Why the node failed in this run, in full (stack trace included), else null. */
export function nodeError(execution: Execution | null, nodeId: string): string | null {
  const run = execution?.node_runs?.[nodeId]
  if (run?.status !== 'error') return null
  if (run.error) return run.error
  const legacy = (execution?.node_outputs[nodeId]?.[0]?.json as Record<string, unknown> | undefined)?.error
  return typeof legacy === 'string' ? legacy : 'The node failed.'
}

/** The columns of a table of items: their top-level keys, in first-seen order. */
export function tableColumns(items: Item[], limit = 50): string[] {
  const seen = new Set<string>()
  for (const it of items.slice(0, limit)) {
    if (it.json && typeof it.json === 'object' && !Array.isArray(it.json)) for (const k of Object.keys(it.json)) seen.add(k)
  }
  return [...seen]
}

/** A cell: text as it is, anything else as short JSON. */
export function cellText(v: unknown): string {
  if (v === undefined) return ''
  if (typeof v === 'string') return v
  const s = JSON.stringify(v)
  return s.length > 120 ? `${s.slice(0, 117)}…` : s
}
