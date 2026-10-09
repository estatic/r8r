import { describe, expect, it } from 'vitest'
import { cellText, nodeError, nodeInput, nodeOutputs, tableColumns } from './nodeData'
import type { Execution, Workflow } from '../types/domain'

const it1 = (json: Record<string, unknown>) => ({ json, binary: {} })
const wf: Workflow = {
  id: 'w',
  name: 'w',
  active: false,
  nodes: [],
  connections: [
    { from_node: 'a', from_output: 1, to_node: 'b', to_input: 0, error: false },
    { from_node: 'c', from_output: 0, to_node: 'b', to_input: 1, error: true },
  ],
  created_at: '',
  updated_at: '',
} as unknown as Workflow
const exec = (runs: Execution['node_runs'], outputs: Execution['node_outputs'] = {}): Execution =>
  ({ id: 'e', workflow_id: 'w', status: 'Success', mode: 'Manual', node_outputs: outputs, node_runs: runs, started_at: '', finished_at: null })

describe('node view data', () => {
  it('shows the recorded input, else what the inputs were sent', () => {
    expect(nodeInput(exec({ b: { status: 'success', counts: {}, input: [it1({ x: 1 })] } }), wf, 'b')).toEqual([it1({ x: 1 })])
    const derived = exec({
      a: { status: 'success', counts: {}, outputs: [[it1({ no: 1 })], [it1({ yes: 1 })]] },
      c: { status: 'error', counts: {}, error_items: [it1({ error: 'x' })] },
    })
    expect(nodeInput(derived, wf, 'b')).toEqual([it1({ yes: 1 }), it1({ error: 'x' })])
    expect(nodeInput(exec({}), wf, 'b')).toBeNull()
  })

  it('names outputs by the node type and adds the error output', () => {
    const e = exec({ i: { status: 'success', counts: {}, outputs: [[it1({ a: 1 })], []], error_items: [it1({ error: 'e' })] } })
    expect(nodeOutputs(e, 'i', ['true', 'false'])!.map((p) => [p.label, p.items.length])).toEqual([['true', 1], ['false', 0], ['error', 1]])
    expect(nodeOutputs(exec({}, { old: [it1({ a: 1 })] }), 'old', ['main'])).toEqual([{ label: 'Output', items: [it1({ a: 1 })] }])
    expect(nodeOutputs(exec({}), 'none', [])).toBeNull()
  })

  it('gives the full error, from older runs too', () => {
    expect(nodeError(exec({ c: { status: 'error', counts: {}, error: 'boom\n  at main (line 2:5)' } }), 'c')).toBe('boom\n  at main (line 2:5)')
    expect(nodeError(exec({ c: { status: 'error', counts: {} } }, { c: [it1({ error: 'old' })] }), 'c')).toBe('old')
    expect(nodeError(exec({ c: { status: 'success', counts: {} } }), 'c')).toBeNull()
  })

  it('builds table columns and cells', () => {
    expect(tableColumns([it1({ a: 1, b: 2 }), it1({ c: 3, a: 4 })])).toEqual(['a', 'b', 'c'])
    expect(cellText('hi')).toBe('hi')
    expect(cellText({ n: 1 })).toBe('{"n":1}')
    expect(cellText(undefined)).toBe('')
  })
})
