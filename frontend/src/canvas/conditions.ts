/** If / Filter conditions: rules joined by AND or OR (the node's `conditions`). */

export interface ConditionRule {
  /** Usually an expression picked from the input, e.g. `{{ $json.count }}`. */
  left: string
  operator: string
  right: string
}

export interface ConditionsForm {
  combinator: 'and' | 'or'
  rules: ConditionRule[]
}

/** The operators the engine knows; `unary` ones take no value to compare to. */
export const OPERATORS: { value: string; label: string; unary?: boolean; needsRight?: boolean }[] = [
  { value: 'equals', label: 'is equal to' },
  { value: 'notEquals', label: 'is not equal to' },
  { value: 'contains', label: 'contains' },
  { value: 'notContains', label: 'does not contain' },
  { value: 'startsWith', label: 'starts with' },
  { value: 'endsWith', label: 'ends with' },
  { value: 'matchesRegex', label: 'matches regex', needsRight: true },
  { value: 'gt', label: 'is greater than', needsRight: true },
  { value: 'gte', label: 'is greater than or equal to', needsRight: true },
  { value: 'lt', label: 'is less than', needsRight: true },
  { value: 'lte', label: 'is less than or equal to', needsRight: true },
  { value: 'isEmpty', label: 'is empty', unary: true },
  { value: 'isNotEmpty', label: 'is not empty', unary: true },
  { value: 'isTrue', label: 'is true', unary: true },
  { value: 'isFalse', label: 'is false', unary: true },
  { value: 'exists', label: 'exists', unary: true },
  { value: 'notExists', label: 'does not exist', unary: true },
]

export const isUnary = (operator: string) => OPERATORS.find((o) => o.value === operator)?.unary === true

export const emptyRule = (): ConditionRule => ({ left: '', operator: 'equals', right: '' })

const str = (v: unknown) => (v === undefined || v === null ? '' : typeof v === 'string' ? v : JSON.stringify(v))

/** The form for a node's parameters; an older `condition` becomes an "is true" rule. */
export function loadConditions(p: Record<string, unknown>): ConditionsForm {
  const c = p.conditions as { combinator?: unknown; rules?: unknown } | undefined
  if (c && typeof c === 'object' && Array.isArray(c.rules)) {
    return {
      combinator: c.combinator === 'or' ? 'or' : 'and',
      rules: (c.rules as Record<string, unknown>[]).map((r) => ({ left: str(r.left), operator: str(r.operator) || 'equals', right: str(r.right) })),
    }
  }
  if (p.condition !== undefined) return { combinator: 'and', rules: [{ left: str(p.condition), operator: 'isTrue', right: '' }] }
  return { combinator: 'and', rules: [emptyRule()] }
}

/** The node's `conditions`, or why the form can't be saved. */
export function buildConditions(f: ConditionsForm): { conditions: Record<string, unknown> } | { error: string } {
  if (f.rules.length === 0) return { error: 'Add at least one condition.' }
  const rules = []
  for (const [i, r] of f.rules.entries()) {
    const op = OPERATORS.find((o) => o.value === r.operator)
    if (!op) return { error: `Condition ${i + 1}: choose an operator.` }
    if (!r.left.trim()) return { error: `Condition ${i + 1}: choose the value to check.` }
    if (op.needsRight && !r.right.trim()) return { error: `Condition ${i + 1}: "${op.label}" needs a value to compare to.` }
    rules.push(op.unary ? { left: r.left, operator: r.operator } : { left: r.left, operator: r.operator, right: r.right })
  }
  return { conditions: { combinator: f.combinator, rules } }
}
