/** One row of the Set node's form. */
export interface SetFieldRow {
  name: string
  /** `fixed`: the value as typed; `expression`: picked or written from the input. */
  mode: 'fixed' | 'expression'
  type: 'string' | 'number' | 'boolean' | 'json'
  text: string
}

/** The rows for a Set node's stored `fields`. */
export function loadRows(fields: unknown): SetFieldRow[] {
  if (!fields || typeof fields !== 'object' || Array.isArray(fields)) return []
  return Object.entries(fields as Record<string, unknown>).map(([name, value]): SetFieldRow => {
    if (typeof value === 'string') {
      return { name, mode: value.includes('{{') ? 'expression' : 'fixed', type: 'string', text: value }
    }
    if (typeof value === 'number') return { name, mode: 'fixed', type: 'number', text: String(value) }
    if (typeof value === 'boolean') return { name, mode: 'fixed', type: 'boolean', text: String(value) }
    return { name, mode: 'fixed', type: 'json', text: JSON.stringify(value) }
  })
}

/** The `fields` the rows describe, or why they can't be saved. */
export function buildFields(rows: SetFieldRow[]): { fields: Record<string, unknown> } | { error: string } {
  const fields: Record<string, unknown> = {}
  for (const row of rows) {
    const name = row.name.trim()
    if (!name) return { error: 'Every field needs a name.' }
    if (name in fields) return { error: `The field name "${name}" is used twice.` }
    if (row.mode === 'expression' || row.type === 'string') {
      fields[name] = row.text
    } else if (row.type === 'number') {
      const n = Number(row.text)
      if (row.text.trim() === '' || Number.isNaN(n)) return { error: `"${name}" must be a number.` }
      fields[name] = n
    } else if (row.type === 'boolean') {
      fields[name] = row.text === 'true'
    } else {
      try {
        fields[name] = JSON.parse(row.text)
      } catch {
        return { error: `"${name}" must be valid JSON.` }
      }
    }
  }
  return { fields }
}
