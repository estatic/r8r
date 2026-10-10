import type { Execution, Workflow } from '../types/domain'
import { nodeRef } from './nodeNames'

export interface FieldPath {
  /** JS path from the item root, e.g. `.message.chat.id` or `["odd key"]`. */
  path: string
  /** The value in the run it was taken from, as JSON. */
  preview: string
  /** The same path as keys and indexes, to write it in other syntaxes. */
  segments: (string | number)[]
}

export interface PickableField extends FieldPath {
  /** What the value box gets, e.g. `{{ $json.message.chat.id }}`. */
  expression: string
}

export interface UpstreamSource {
  nodeId: string
  /** How expressions read it: `$("name")`. */
  name: string
  /** Connected straight to the node's input: its fields are `$json`. */
  direct: boolean
  /** From its first output item in the run; empty when it hasn't run. */
  fields: PickableField[]
}

const IDENTIFIER = /^[A-Za-z_$][\w$]*$/

function step(key: string): string {
  return IDENTIFIER.test(key) ? `.${key}` : `[${JSON.stringify(key)}]`
}

/** Every leaf of `value` (arrays through their first element), in order. */
export function fieldPaths(value: unknown, base = '', segments: (string | number)[] = []): FieldPath[] {
  if (Array.isArray(value)) {
    return value.length > 0 ? fieldPaths(value[0], `${base}[0]`, [...segments, 0]) : [{ path: base, preview: '[]', segments }]
  }
  if (value !== null && typeof value === 'object') {
    const entries = Object.entries(value as Record<string, unknown>)
    if (entries.length === 0) return base ? [{ path: base, preview: '{}', segments }] : []
    return entries.flatMap(([k, v]) => fieldPaths(v, base + step(k), [...segments, k]))
  }
  return [{ path: base, preview: JSON.stringify(value) ?? String(value), segments }]
}

/**
 * How a Code node's script reads a field: from its input (`$json` /
 * `_json`) or from an earlier node by name (`$("name").json` /
 * `_node["name"]["json"]`).
 */
export function codeReference(language: 'javaScript' | 'python', direct: boolean, nodeName: string, segments: (string | number)[]): string {
  if (language === 'python') {
    const root = direct ? '_json' : `_node[${JSON.stringify(nodeName)}]["json"]`
    return root + segments.map((s) => `[${JSON.stringify(s)}]`).join('')
  }
  const root = direct ? '$json' : `${nodeRef({ id: nodeName, name: nodeName })}.json`
  return root + segments.map((s) => (typeof s === 'number' ? `[${s}]` : step(s))).join('')
}

/**
 * The nodes whose data reaches `nodeId`, nearest first: those connected to
 * its input (read as `$json`, the item it receives), then the ones before
 * them (read by name, `$("name")`). Fields come from `execution`.
 */
export function upstreamSources(workflow: Workflow, nodeId: string, execution: Execution | null): UpstreamSource[] {
  const seen = new Set<string>([nodeId])
  const sources: UpstreamSource[] = []
  let frontier = [nodeId]
  let direct = true
  while (frontier.length > 0) {
    const next: string[] = []
    for (const target of frontier) {
      for (const c of workflow.connections) {
        if (c.to_node !== target || seen.has(c.from_node)) continue
        seen.add(c.from_node)
        next.push(c.from_node)
        const item = execution?.node_outputs[c.from_node]?.[0]?.json
        const from = workflow.nodes.find((n) => n.id === c.from_node)
        const name = from?.name ?? c.from_node
        const root = direct ? '$json' : `${nodeRef({ id: c.from_node, name })}.json`
        sources.push({
          nodeId: c.from_node,
          name,
          direct,
          fields: item === undefined ? [] : fieldPaths(item).map((f) => ({ ...f, expression: `{{ ${root}${f.path} }}` })),
        })
      }
    }
    frontier = next
    direct = false
  }
  return sources
}
