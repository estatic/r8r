import { describe, expect, it } from 'vitest'
import { buildConditions, loadConditions } from './conditions'

describe('If / Filter conditions', () => {
  it('starts a new node with one empty rule', () => {
    expect(loadConditions({})).toEqual({ combinator: 'and', rules: [{ left: '', operator: 'equals', right: '' }] })
  })

  it('round-trips stored conditions', () => {
    const conditions = {
      combinator: 'or',
      rules: [
        { left: '{{ $json.count }}', operator: 'gt', right: '5' },
        { left: '{{ $json.name }}', operator: 'isNotEmpty' },
      ],
    }
    expect(buildConditions(loadConditions({ conditions }))).toEqual({ conditions })
  })

  it('turns an older single condition into an "is true" rule', () => {
    expect(loadConditions({ condition: '{{ $json.ok }}' }).rules).toEqual([{ left: '{{ $json.ok }}', operator: 'isTrue', right: '' }])
    expect(loadConditions({ condition: true }).rules[0].left).toBe('true')
  })

  it('says what is missing', () => {
    expect(buildConditions({ combinator: 'and', rules: [] })).toEqual({ error: 'Add at least one condition.' })
    expect(buildConditions({ combinator: 'and', rules: [{ left: '', operator: 'equals', right: 'x' }] })).toEqual({
      error: 'Condition 1: choose the value to check.',
    })
    expect(buildConditions({ combinator: 'and', rules: [{ left: '{{ $json.n }}', operator: 'gt', right: ' ' }] })).toEqual({
      error: 'Condition 1: "is greater than" needs a value to compare to.',
    })
  })
})
