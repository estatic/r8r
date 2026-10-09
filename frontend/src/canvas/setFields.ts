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

/** The Set node's other settings: mode, which input fields to keep, dot notation. */
export interface SetOptions {
  mode: 'manual' | 'json'
  jsonOutput: string
  include: 'all' | 'none' | 'selected' | 'except'
  includeFields: string
  dotNotation: boolean
}

export const SET_OPTION_KEYS = ['mode', 'json_output', 'include', 'include_fields', 'dot_notation']

export function loadSetOptions(p: Record<string, unknown>): SetOptions {
  const include = ['none', 'selected', 'except'].includes(p.include as string) ? (p.include as SetOptions['include']) : 'all'
  const jo = p.json_output
  return {
    mode: p.mode === 'json' ? 'json' : 'manual',
    jsonOutput: typeof jo === 'string' ? jo : jo !== undefined ? JSON.stringify(jo, null, 2) : '{\n  "my_field": "value"\n}',
    include,
    includeFields: Array.isArray(p.include_fields) ? p.include_fields.join(', ') : typeof p.include_fields === 'string' ? p.include_fields : '',
    dotNotation: p.dot_notation !== false,
  }
}

/** The settings as parameters; defaults (manual, keep all, dot notation on) aren't stored. */
export function buildSetOptions(o: SetOptions): { fields: Record<string, unknown> } | { error: string } {
  const fields: Record<string, unknown> = {}
  if (o.mode === 'json') {
    const text = o.jsonOutput.trim()
    if (!text) return { error: 'Enter the JSON object to set.' }
    // Expressions make it text until the run; only plain JSON is checked here.
    if (!text.includes('{{')) {
      try {
        const v = JSON.parse(text)
        if (!v || typeof v !== 'object' || Array.isArray(v)) return { error: 'The JSON must be an object, e.g. {"name": "value"}.' }
      } catch {
        return { error: 'The JSON isn\'t valid.' }
      }
    }
    fields.mode = 'json'
    fields.json_output = text
  }
  if (o.include !== 'all') {
    fields.include = o.include
    if (o.include === 'selected' || o.include === 'except') {
      const names = o.includeFields.split(',').map((s) => s.trim()).filter(Boolean)
      if (names.length === 0) return { error: 'List the input fields, separated by commas.' }
      fields.include_fields = names
    }
  }
  if (!o.dotNotation) fields.dot_notation = false
  return { fields }
}
