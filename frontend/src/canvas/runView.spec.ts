import { describe, it, expect } from 'vitest'
import { edgeRunState, itemsLabel, nodeRunState, nodeShape } from './runView'
import type { Connection, Execution, NodeRun } from '../types/domain'

function exec(runs: Record<string, NodeRun>): Execution {
  return { id: 'e', workflow_id: 'w', status: 'Running', mode: 'Manual', node_outputs: {}, node_runs: runs, started_at: '', finished_at: null }
}
const conn = (from: string, to: string, from_output = 0, error = false): Connection => ({ from_node: from, from_output, to_node: to, to_input: 0, error })

describe('nodeRunState', () => {
  it('is null without an execution, so the canvas looks as usual', () => {
    expect(nodeRunState(null, 'a')).toBeNull()
  })
  it('maps each node outcome; nodes that have not run (or were skipped) are pending', () => {
    const e = exec({ a: { status: 'success', counts: {} }, b: { status: 'running', counts: {} }, c: { status: 'error', counts: {} }, d: { status: 'skipped', counts: {} } })
    expect(nodeRunState(e, 'a')).toBe('success')
    expect(nodeRunState(e, 'b')).toBe('running')
    expect(nodeRunState(e, 'c')).toBe('error')
    expect(nodeRunState(e, 'd')).toBe('pending')
    expect(nodeRunState(e, 'zzz')).toBe('pending')
  })
})

describe('edgeRunState', () => {
  it('is green once items went through, yellow while the next node runs, red on the error output, grey otherwise', () => {
    const e = exec({
      a: { status: 'success', counts: { '0': 2, '1': 0 } },
      b: { status: 'running', counts: {} },
      c: { status: 'error', counts: { error: 1 } },
    })
    expect(edgeRunState(null, conn('a', 'b'))).toBeNull()
    expect(edgeRunState(e, conn('a', 'b'))).toBe('running')
    expect(edgeRunState(e, conn('a', 'x'))).toBe('success')
    expect(edgeRunState(e, conn('a', 'x', 1))).toBe('pending') // untaken branch
    expect(edgeRunState(e, conn('c', 'x', 0, true))).toBe('error')
    expect(edgeRunState(e, conn('q', 'x'))).toBe('pending') // source never ran
  })
})

describe('itemsLabel', () => {
  it('counts the items an output sent, like n8n; nothing for none', () => {
    const e = exec({ a: { status: 'success', counts: { '0': 1, '1': 3, '2': 0 } } })
    expect(itemsLabel(e, conn('a', 'b'))).toBe('1 item')
    expect(itemsLabel(e, conn('a', 'b', 1))).toBe('3 items')
    expect(itemsLabel(e, conn('a', 'b', 2))).toBeUndefined()
    expect(itemsLabel(null, conn('a', 'b'))).toBeUndefined()
  })
})

describe('nodeShape', () => {
  const connections = [conn('t', 'mid'), conn('mid', 'end')]
  it('draws triggers as starts, nodes with no outgoing link as ends, the rest as boxes', () => {
    expect(nodeShape('t', 'trigger', connections)).toBe('start')
    expect(nodeShape('mid', 'action', connections)).toBe('box')
    expect(nodeShape('end', 'action', connections)).toBe('end')
    expect(nodeShape('lonely-trigger', 'trigger', [])).toBe('start')
  })
  it('counts an error link as outgoing', () => {
    expect(nodeShape('a', 'action', [conn('a', 'b', 0, true)])).toBe('box')
  })
})
