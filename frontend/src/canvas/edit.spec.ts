import { describe, it, expect } from 'vitest'
import { insertNodeIntoConnection, removeConnection, removeNode } from './edit'
import type { Connection, NodeInstance, Workflow } from '../types/domain'

const n = (id: string, x = 0, y = 0): NodeInstance => ({ id, node_type: 'core.set', position: [x, y], parameters: {}, disabled: false })
const c = (from: string, to: string, extra: Partial<Connection> = {}): Connection => ({ from_node: from, from_output: 0, to_node: to, to_input: 0, error: false, ...extra })
const wf = (nodes: NodeInstance[], connections: Connection[]) => ({ id: 'w', name: 'w', active: false, nodes, connections, created_at: '', updated_at: '' }) as Workflow

describe('removeNode', () => {
  it('drops the node and every link to or from it', () => {
    const w = wf([n('a'), n('b'), n('c')], [c('a', 'b'), c('b', 'c'), c('a', 'c')])
    removeNode(w, 'b')
    expect(w.nodes.map((x) => x.id)).toEqual(['a', 'c'])
    expect(w.connections).toEqual([c('a', 'c')])
  })
})

describe('removeConnection', () => {
  it('drops only that link', () => {
    const w = wf([n('a'), n('b')], [c('a', 'b'), c('a', 'b', { error: true })])
    removeConnection(w, c('a', 'b', { error: true }))
    expect(w.connections).toEqual([c('a', 'b')])
  })
})

describe('insertNodeIntoConnection', () => {
  it('puts the node between the two, halfway, keeping the source output and target input', () => {
    const link = c('if', 'b', { from_output: 1, to_input: 0 })
    const w = wf([n('if', 0, 0), n('b', 400, 100)], [c('x', 'if'), link])
    insertNodeIntoConnection(w, link, n('new'))
    expect(w.nodes.find((x) => x.id === 'new')!.position).toEqual([200, 50])
    expect(w.connections).toEqual([c('x', 'if'), c('if', 'new', { from_output: 1 }), c('new', 'b')])
  })

  it('keeps an error route on the first half', () => {
    const link = c('a', 'b', { error: true })
    const w = wf([n('a'), n('b', 200)], [link])
    insertNodeIntoConnection(w, link, n('new'))
    expect(w.connections).toEqual([c('a', 'new', { error: true }), c('new', 'b')])
  })
})
