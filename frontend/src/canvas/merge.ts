/** The Merge node's form, and its stored parameters. */

export interface FieldPair {
  field1: string
  field2: string
}

export interface MergeForm {
  mode: 'append' | 'combine' | 'chooseBranch'
  combineBy: 'matchingFields' | 'position' | 'allCombinations'
  matchFields: FieldPair[]
  joinMode: string
  includeUnpaired: boolean
  clash: 'preferInput2' | 'preferInput1'
  outputInput: 'input1' | 'input2'
}

export const JOIN_MODES = [
  { value: 'keepMatches', label: 'Keep matches (both inputs merged)' },
  { value: 'keepNonMatches', label: 'Keep non-matches' },
  { value: 'keepEverything', label: 'Keep everything' },
  { value: 'enrichInput1', label: 'Enrich input 1 (all of input 1, with matches added)' },
  { value: 'enrichInput2', label: 'Enrich input 2 (all of input 2, with matches added)' },
]

export const MERGE_KEYS = ['mode', 'combine_by', 'match_fields', 'join_mode', 'include_unpaired', 'clash', 'output_input', 'key']

export function loadMerge(p: Record<string, unknown>): MergeForm {
  const pairs = Array.isArray(p.match_fields)
    ? (p.match_fields as Record<string, unknown>[]).map((m) => ({ field1: String(m.field1 ?? ''), field2: String(m.field2 ?? '') }))
    : []
  // Saved before: merge by one key, the same name in both inputs.
  const legacyKey = p.mode === 'mergeByKey' && typeof p.key === 'string' ? [{ field1: p.key, field2: p.key }] : []
  const mode = p.mode === 'combine' || p.mode === 'mergeByKey' ? 'combine' : p.mode === 'chooseBranch' ? 'chooseBranch' : 'append'
  const combineBy = p.combine_by === 'position' || p.combine_by === 'allCombinations' ? p.combine_by : 'matchingFields'
  return {
    mode,
    combineBy,
    matchFields: pairs.length > 0 ? pairs : legacyKey.length > 0 ? legacyKey : [{ field1: '', field2: '' }],
    joinMode: typeof p.join_mode === 'string' ? p.join_mode : p.mode === 'mergeByKey' ? 'keepEverything' : 'keepMatches',
    includeUnpaired: p.include_unpaired === true,
    clash: p.clash === 'preferInput1' ? 'preferInput1' : 'preferInput2',
    outputInput: p.output_input === 'input2' ? 'input2' : 'input1',
  }
}

export function buildMerge(f: MergeForm): { fields: Record<string, unknown> } | { error: string } {
  if (f.mode === 'append') return { fields: { mode: 'append' } }
  if (f.mode === 'chooseBranch') return { fields: { mode: 'chooseBranch', output_input: f.outputInput } }
  const fields: Record<string, unknown> = { mode: 'combine', combine_by: f.combineBy }
  if (f.combineBy === 'matchingFields') {
    const pairs = f.matchFields.map((m) => ({ field1: m.field1.trim(), field2: m.field2.trim() })).filter((m) => m.field1 || m.field2)
    if (pairs.length === 0) return { error: 'Choose the fields to match: one from input 1 and one from input 2.' }
    if (pairs.some((m) => !m.field1 || !m.field2)) return { error: 'Each match needs a field from input 1 and one from input 2.' }
    fields.match_fields = pairs
    fields.join_mode = f.joinMode
  }
  if (f.combineBy === 'position' && f.includeUnpaired) fields.include_unpaired = true
  if (f.clash === 'preferInput1') fields.clash = 'preferInput1'
  return { fields }
}
