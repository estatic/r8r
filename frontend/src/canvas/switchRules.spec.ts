import { describe, expect, it } from 'vitest'
import { buildSwitch, loadSwitch } from './switchRules'

const cond = (left: string, operator: string, right: string) => ({ combinator: 'and', rules: [{ left, operator, right }] })

describe('Switch form', () => {
  it('starts with one empty rule', () => {
    expect(loadSwitch({}).rules).toHaveLength(1)
    expect(loadSwitch({}).mode).toBe('rules')
  })

  it('round-trips rules with names, a fallback and all-matching', () => {
    const fields = {
      mode: 'rules',
      rules: [{ output_name: 'urgent', conditions: cond('{{ $json.p }}', 'gt', '5') }, { conditions: cond('{{ $json.p }}', 'lte', '5') }],
      fallback_output: 'extra',
      all_matching_outputs: true,
    }
    expect(buildSwitch(loadSwitch(fields))).toEqual({ fields })
    expect(buildSwitch(loadSwitch({ ...fields, fallback_output: 1 }))).toEqual({ fields: { ...fields, fallback_output: 1 } })
  })

  it('round-trips expression mode', () => {
    const fields = { mode: 'expression', number_outputs: 3, output: '{{ $json.kind }}' }
    expect(buildSwitch(loadSwitch(fields))).toEqual({ fields })
    expect(buildSwitch({ ...loadSwitch(fields), output: '7' })).toEqual({ error: "Output 7 doesn't exist (0 to 2)." })
  })

  it('turns value/cases into equal-to rules with a fallback for the default', () => {
    const form = loadSwitch({ value: '{{ $json.kind }}', cases: ['a', 'b'] })
    expect(buildSwitch(form)).toEqual({
      fields: { mode: 'rules', rules: [{ conditions: cond('{{ $json.kind }}', 'equals', 'a') }, { conditions: cond('{{ $json.kind }}', 'equals', 'b') }], fallback_output: 'extra' },
    })
  })

  it('names the rule that is incomplete', () => {
    expect(buildSwitch(loadSwitch({}))).toEqual({ error: 'Rule 1: Condition 1: choose the value to check.' })
  })
})
