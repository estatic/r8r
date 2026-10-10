import { describe, it, expect } from 'vitest'
import { codeReference, fieldPaths, upstreamSources } from './inputData'
import type { Connection, Execution, NodeInstance, Workflow } from '../types/domain'

const n = (id: string, node_type = 'core.set'): NodeInstance => ({ id, node_type, position: [0, 0], parameters: {}, disabled: false })
const c = (from: string, to: string): Connection => ({ from_node: from, from_output: 0, to_node: to, to_input: 0, error: false })
const wf = (nodes: NodeInstance[], connections: Connection[]) => ({ id: 'w', name: 'w', active: false, nodes, connections, created_at: '', updated_at: '' }) as Workflow
const exec = (outputs: Record<string, unknown[]>): Execution => ({
  id: 'e', workflow_id: 'w', status: 'Success', mode: 'Manual', started_at: '', finished_at: null,
  node_outputs: Object.fromEntries(Object.entries(outputs).map(([k, items]) => [k, items.map((json) => ({ json, binary: {} }))])),
})

describe('fieldPaths', () => {
  it('lists every leaf with a JS path, quoting keys that need it, and the first array element', () => {
    expect(fieldPaths({ message: { chat: { id: 42 }, text: 'hi' }, 'odd key': true, list: [{ a: 1 }], empty: {} })).toEqual([
      { path: '.message.chat.id', preview: '42', segments: ['message', 'chat', 'id'] },
      { path: '.message.text', preview: '"hi"', segments: ['message', 'text'] },
      { path: '["odd key"]', preview: 'true', segments: ['odd key'] },
      { path: '.list[0].a', preview: '1', segments: ['list', 0, 'a'] },
      { path: '.empty', preview: '{}', segments: ['empty'] },
    ])
  })
})

describe('upstreamSources', () => {
  const w = wf([n('tg', 'telegram.trigger'), n('agent', 'ai.agent'), n('set')], [c('tg', 'agent'), c('agent', 'set')])
  const e = exec({ tg: [{ message: { chat: { id: 7 } } }], agent: [{ response: 'ok', message: { chat: { id: 7 } } }] })

  it('offers the connected node as $json and earlier nodes by name, nearest first', () => {
    const sources = upstreamSources(w, 'set', e)
    expect(sources.map((s) => [s.nodeId, s.direct])).toEqual([['agent', true], ['tg', false]])
    expect(sources[0].fields.map((f) => f.expression)).toEqual(['{{ $json.response }}', '{{ $json.message.chat.id }}'])
    expect(sources[1].fields.map((f) => f.expression)).toEqual(['{{ $("tg").json.message.chat.id }}'])
    const named = { ...w, nodes: w.nodes.map((x) => (x.id === 'tg' ? { ...x, name: 'Telegram Trigger' } : x)) }
    expect(upstreamSources(named, 'set', e)[1].fields[0].expression).toBe('{{ $("Telegram Trigger").json.message.chat.id }}')
  })

  it('lists a node that has not run yet with no fields', () => {
    const sources = upstreamSources(w, 'set', exec({}))
    expect(sources.map((s) => [s.nodeId, s.fields.length])).toEqual([['agent', 0], ['tg', 0]])
  })

  it('has nothing for a start node', () => {
    expect(upstreamSources(w, 'tg', e)).toEqual([])
  })
})

describe('codeReference', () => {
  const segments = ['message', 'chat', 'id']
  it('writes JavaScript references, for the input and for an earlier node', () => {
    expect(codeReference('javaScript', true, 'agent', segments)).toBe('$json.message.chat.id')
    expect(codeReference('javaScript', false, 'Telegram Trigger', segments)).toBe('$("Telegram Trigger").json.message.chat.id')
    expect(codeReference('javaScript', true, 'a', ['odd key', 0, 'x'])).toBe('$json["odd key"][0].x')
  })
  it('writes Python references with subscripts', () => {
    expect(codeReference('python', true, 'agent', segments)).toBe('_json["message"]["chat"]["id"]')
    expect(codeReference('python', false, 'tg', ['list', 0])).toBe('_node["tg"]["json"]["list"][0]')
  })
})
