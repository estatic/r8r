import { describe, it, expect } from 'vitest'
import { buildFields, buildSetOptions, loadRows, loadSetOptions } from './setFields'

describe('loadRows', () => {
  it('reads each stored value as a fixed value of its type, or an expression', () => {
    expect(loadRows({ chat_id: '{{ $json.message.chat.id }}', greeting: 'hi', n: 3, ok: false, meta: { a: [1] } })).toEqual([
      { name: 'chat_id', mode: 'expression', type: 'string', text: '{{ $json.message.chat.id }}' },
      { name: 'greeting', mode: 'fixed', type: 'string', text: 'hi' },
      { name: 'n', mode: 'fixed', type: 'number', text: '3' },
      { name: 'ok', mode: 'fixed', type: 'boolean', text: 'false' },
      { name: 'meta', mode: 'fixed', type: 'json', text: '{"a":[1]}' },
    ])
  })
  it('has no rows for no fields', () => {
    expect(loadRows(undefined)).toEqual([])
  })
})

describe('buildFields', () => {
  it('turns rows back into typed values', () => {
    const rows = loadRows({ chat_id: '{{ $json.message.chat.id }}', greeting: 'hi', n: 3, ok: false, meta: { a: [1] } })
    expect(buildFields(rows)).toEqual({ fields: { chat_id: '{{ $json.message.chat.id }}', greeting: 'hi', n: 3, ok: false, meta: { a: [1] } } })
  })
  it('trims names and refuses blank or repeated ones', () => {
    expect(buildFields([{ name: ' a ', mode: 'fixed', type: 'string', text: 'x' }])).toEqual({ fields: { a: 'x' } })
    expect(buildFields([{ name: ' ', mode: 'fixed', type: 'string', text: 'x' }])).toEqual({ error: 'Every field needs a name.' })
    expect(buildFields([
      { name: 'a', mode: 'fixed', type: 'string', text: 'x' },
      { name: 'a', mode: 'fixed', type: 'string', text: 'y' },
    ])).toEqual({ error: 'The field name "a" is used twice.' })
  })
  it('refuses a number or JSON that does not parse', () => {
    expect(buildFields([{ name: 'n', mode: 'fixed', type: 'number', text: 'twelve' }])).toEqual({ error: '"n" must be a number.' })
    expect(buildFields([{ name: 'j', mode: 'fixed', type: 'json', text: '{oops' }])).toEqual({ error: '"j" must be valid JSON.' })
  })
})

describe('Set options', () => {
  it('stores nothing for the defaults', () => {
    expect(buildSetOptions(loadSetOptions({}))).toEqual({ fields: {} })
  })

  it('round-trips JSON mode, kept fields and dot notation off', () => {
    const fields = { mode: 'json', json_output: '{"id": {{ $json.id }}}', include: 'selected', include_fields: ['chat_id', 'text'], dot_notation: false }
    expect(buildSetOptions(loadSetOptions(fields))).toEqual({ fields })
  })

  it('checks plain JSON and the field list', () => {
    expect(buildSetOptions({ ...loadSetOptions({}), mode: 'json', jsonOutput: '[1]' })).toHaveProperty('error')
    expect(buildSetOptions({ ...loadSetOptions({}), include: 'except', includeFields: ' ' })).toEqual({ error: 'List the input fields, separated by commas.' })
  })
})
