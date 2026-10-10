import type { NodeInstance, Workflow } from '../types/domain'

/**
 * `base`, or `base 2`, `base 3`, ... -- the first no other node of
 * `workflow` is called (the server names unnamed nodes the same way).
 */
export function uniqueName(workflow: Workflow, base: string, exceptId?: string): string {
  const taken = new Set(workflow.nodes.filter((n) => n.id !== exceptId).map((n) => n.name))
  let name = base
  for (let n = 2; taken.has(name); n++) name = `${base} ${n}`
  return name
}

/** How an expression or a Code node reads a node: `$("Get users")`. */
export function nodeRef(node: Pick<NodeInstance, 'id' | 'name'>): string {
  return `$(${JSON.stringify(node.name ?? node.id)})`
}

function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

/**
 * Every way a parameter can point at the node called `name`: `$("name")`,
 * `$node["name"]` and Python's `_node["name"]`, with any quote.
 */
function referencePattern(name: string): RegExp {
  const quoted = ['"', "'", '`'].map((q) => `${q}${escapeRegExp(name.replace(/\\/g, '\\\\').split(q).join(`\\${q}`))}${q}`)
  return new RegExp(`(\\$\\(\\s*|\\$node\\[\\s*|_node\\[\\s*)(?:${quoted.join('|')})`, 'g')
}

function rewrite(value: unknown, from: RegExp, to: string): unknown {
  if (typeof value === 'string') return value.replace(from, (_m, prefix: string) => prefix + to)
  if (Array.isArray(value)) return value.map((v) => rewrite(v, from, to))
  if (value !== null && typeof value === 'object') {
    return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, rewrite(v, from, to)]))
  }
  return value
}

/**
 * Renames the node `nodeId` (to a name no other node has) and points every
 * reference to its old name in the workflow's parameters at the new one,
 * as n8n does. Returns the name it got.
 */
export function renameNode(workflow: Workflow, nodeId: string, wanted: string): string {
  const node = workflow.nodes.find((n) => n.id === nodeId)
  if (!node) return wanted
  const old = node.name
  const name = uniqueName(workflow, wanted.trim() || old || node.node_type, nodeId)
  node.name = name
  if (old && old !== name) {
    const from = referencePattern(old)
    const to = JSON.stringify(name)
    for (const n of workflow.nodes) n.parameters = rewrite(n.parameters, from, to) as Record<string, unknown>
  }
  return name
}
