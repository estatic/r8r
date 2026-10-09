/** The Switch node's form: rules (one output each) or an expression. */
import { buildConditions, loadConditions, type ConditionsForm } from './conditions'

export interface SwitchRule {
  outputName: string
  conditions: ConditionsForm
}

export interface SwitchForm {
  mode: 'rules' | 'expression'
  rules: SwitchRule[]
  /** `none` drops an item that meets no rule; `extra` adds a Fallback output; a number reuses that output. */
  fallback: string
  allMatching: boolean
  numberOutputs: string
  output: string
}

export const SWITCH_KEYS = ['mode', 'rules', 'fallback_output', 'all_matching_outputs', 'number_outputs', 'output', 'value', 'cases']

export const emptySwitchRule = (): SwitchRule => ({ outputName: '', conditions: loadConditions({}) })

const str = (v: unknown) => (v === undefined || v === null ? '' : typeof v === 'string' ? v : JSON.stringify(v))

export function loadSwitch(p: Record<string, unknown>): SwitchForm {
  const base: SwitchForm = { mode: 'rules', rules: [], fallback: 'none', allMatching: p.all_matching_outputs === true, numberOutputs: '4', output: '' }
  if (p.mode === 'expression') {
    return { ...base, mode: 'expression', numberOutputs: str(p.number_outputs ?? 4), output: str(p.output), rules: [emptySwitchRule()] }
  }
  if (Array.isArray(p.rules)) {
    const fb = p.fallback_output
    return {
      ...base,
      rules: (p.rules as Record<string, unknown>[]).map((r) => ({ outputName: str(r.output_name), conditions: loadConditions({ conditions: r.conditions }) })),
      fallback: fb === 'extra' ? 'extra' : typeof fb === 'number' ? String(fb) : 'none',
    }
  }
  // Saved before rules: `value` against `cases`, the last output the default.
  if (Array.isArray(p.cases)) {
    return {
      ...base,
      rules: (p.cases as unknown[]).map((c) => ({
        outputName: '',
        conditions: { combinator: 'and', rules: [{ left: str(p.value), operator: 'equals', right: str(c) }] },
      })),
      fallback: 'extra',
    }
  }
  return { ...base, rules: [emptySwitchRule()] }
}

export function buildSwitch(f: SwitchForm): { fields: Record<string, unknown> } | { error: string } {
  if (f.mode === 'expression') {
    const n = Number(f.numberOutputs)
    if (!Number.isInteger(n) || n < 1 || n > 32) return { error: 'Number of outputs must be between 1 and 32.' }
    if (!f.output.trim()) return { error: 'Enter the output index (a number or an expression).' }
    const output = /^\d+$/.test(f.output.trim()) ? Number(f.output.trim()) : f.output.trim()
    if (typeof output === 'number' && output >= n) return { error: `Output ${output} doesn't exist (0 to ${n - 1}).` }
    return { fields: { mode: 'expression', number_outputs: n, output } }
  }
  if (f.rules.length === 0) return { error: 'Add at least one routing rule.' }
  const rules = []
  for (const [i, r] of f.rules.entries()) {
    const built = buildConditions(r.conditions)
    if ('error' in built) return { error: `Rule ${i + 1}: ${built.error}` }
    rules.push(r.outputName.trim() ? { output_name: r.outputName.trim(), conditions: built.conditions } : { conditions: built.conditions })
  }
  const fields: Record<string, unknown> = { mode: 'rules', rules }
  if (f.fallback === 'extra') fields.fallback_output = 'extra'
  else if (f.fallback !== 'none') {
    const n = Number(f.fallback)
    if (n >= f.rules.length) return { error: 'The fallback output must be one of the rule outputs.' }
    fields.fallback_output = n
  }
  if (f.allMatching) fields.all_matching_outputs = true
  return { fields }
}
