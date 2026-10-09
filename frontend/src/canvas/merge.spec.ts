import { describe, expect, it } from 'vitest'
import { buildMerge, loadMerge } from './merge'

describe('Merge form', () => {
  it('starts as append', () => {
    expect(buildMerge(loadMerge({}))).toEqual({ fields: { mode: 'append' } })
  })

  it('round-trips combine by matching fields', () => {
    const fields = { mode: 'combine', combine_by: 'matchingFields', match_fields: [{ field1: 'id', field2: 'user.id' }], join_mode: 'enrichInput1', clash: 'preferInput1' }
    expect(buildMerge(loadMerge(fields))).toEqual({ fields })
  })

  it('round-trips position, all combinations and choose branch', () => {
    for (const fields of [
      { mode: 'combine', combine_by: 'position', include_unpaired: true },
      { mode: 'combine', combine_by: 'allCombinations' },
      { mode: 'chooseBranch', output_input: 'input2' },
    ]) {
      expect(buildMerge(loadMerge(fields))).toEqual({ fields })
    }
  })

  it('turns an older merge-by-key into matching that keeps everything', () => {
    expect(buildMerge(loadMerge({ mode: 'mergeByKey', key: 'id' }))).toEqual({
      fields: { mode: 'combine', combine_by: 'matchingFields', match_fields: [{ field1: 'id', field2: 'id' }], join_mode: 'keepEverything' },
    })
  })

  it('needs both fields of a match', () => {
    expect(buildMerge({ ...loadMerge({ mode: 'combine' }) })).toHaveProperty('error')
    expect(buildMerge({ ...loadMerge({ mode: 'combine' }), matchFields: [{ field1: 'id', field2: '' }] })).toEqual({
      error: 'Each match needs a field from input 1 and one from input 2.',
    })
  })
})
