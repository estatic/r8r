import { describe, it, expect } from 'vitest'
import { nodeRef, renameNode, uniqueName } from './nodeNames'
import type { NodeInstance, Workflow } from '../types/domain'

function node(id: string, name: string | undefined, parameters: Record<string, unknown> = {}): NodeInstance {
  return { id, node_type: 'core.set', name, position: [0, 0], parameters, disabled: false }
}

function workflow(nodes: NodeInstance[]): Workflow {
  return { id: 'wf', name: 'w', active: false, nodes, connections: [], created_at: '', updated_at: '' }
}

describe('node names', () => {
  it('numbers a name another node already has', () => {
    const wf = workflow([node('a', 'Set'), node('b', 'Set 2')])
    expect(uniqueName(wf, 'Set')).toBe('Set 3')
    expect(uniqueName(wf, 'Code')).toBe('Code')
    expect(uniqueName(wf, 'Set', 'a')).toBe('Set')
  })

  it('reads a node by its name', () => {
    expect(nodeRef(node('a', 'Get users'))).toBe('$("Get users")')
    expect(nodeRef(node('a', 'say "hi"'))).toBe('$("say \\"hi\\"")')
  })

  it('renaming points every reference to the old name at the new one', () => {
    const wf = workflow([
      node('a', 'Get users'),
      node('b', 'Reply', {
        text: '{{ $("Get users").json.name }} / {{ $(\'Get users\').first().json.id }}',
        nested: [{ v: '{{ $node["Get users"].json.x }}' }],
        script: "return _node['Get users']['json']",
        other: '{{ $("Get users 2").json }} and Get users',
      }),
    ])
    expect(renameNode(wf, 'a', 'Load users')).toBe('Load users')
    expect(wf.nodes[0].name).toBe('Load users')
    expect(wf.nodes[1].parameters).toEqual({
      text: '{{ $("Load users").json.name }} / {{ $("Load users").first().json.id }}',
      nested: [{ v: '{{ $node["Load users"].json.x }}' }],
      script: 'return _node["Load users"][\'json\']',
      other: '{{ $("Get users 2").json }} and Get users',
    })
  })

  it('keeps names unique and never empty', () => {
    const wf = workflow([node('a', 'A'), node('b', 'B')])
    expect(renameNode(wf, 'b', 'A')).toBe('A 2')
    expect(renameNode(wf, 'b', '   ')).toBe('A 2')
  })
})
