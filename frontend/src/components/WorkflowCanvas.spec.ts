import { describe, it, expect, beforeAll, beforeEach, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import { createPinia, setActivePinia } from 'pinia'
import WorkflowCanvas, { connectionFromVueFlow, edgeSourceHandle, edgeStyle } from './WorkflowCanvas.vue'
import type { NodeInstance, Connection } from '../types/domain'

beforeAll(() => {
  class ResizeObserverStub {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  ;(globalThis as unknown as { ResizeObserver: typeof ResizeObserverStub }).ResizeObserver = ResizeObserverStub

  // jsdom has no DOMMatrixReadOnly; Vue Flow's updateNodeInternals /
  // updateNodeDimensions path (triggered by our own updateNodeInternals()
  // calls after ports resolve) reads `.m22` off one to compute zoom. A
  // minimal identity-matrix stub is enough for tests to exercise that path
  // without throwing.
  class DOMMatrixReadOnlyStub {
    m22 = 1
    constructor(_transform?: string) {}
  }
  ;(globalThis as unknown as { DOMMatrixReadOnly: typeof DOMMatrixReadOnlyStub }).DOMMatrixReadOnly = DOMMatrixReadOnlyStub
})

const NODE_TYPES = [
  { type_name: 'core.manualTrigger', display_name: 'Manual Trigger', icon: '🖱️', category: 'trigger', description: '', credential_types: [], output_ports: ['main'] },
  { type_name: 'core.if', display_name: 'If', icon: '❓', category: 'flowControl', description: '', credential_types: [], output_ports: ['true', 'false'] },
]

function stubFetch(nodeTypesResponse: unknown = NODE_TYPES, portsResponse: string[] = ['true', 'false'], tools: unknown[] = []) {
  vi.stubGlobal(
    'fetch',
    vi.fn((url: string) => {
      if (url === '/rest/r8r/tools') {
        return Promise.resolve({ ok: true, status: 200, json: async () => tools })
      }
      if (url === '/rest/r8r/node-types') {
        return Promise.resolve({ ok: true, status: 200, json: async () => nodeTypesResponse })
      }
      if (url.includes('/output-ports')) {
        return Promise.resolve({ ok: true, status: 200, json: async () => ({ output_ports: portsResponse }) })
      }
      return Promise.reject(new Error(`unexpected fetch: ${url}`))
    }),
  )
}

async function flush() {
  await new Promise((r) => setTimeout(r, 0))
  await nextTick()
  await new Promise((r) => setTimeout(r, 0))
  await nextTick()
}

describe('WorkflowCanvas', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('renders one source handle per output port plus one error handle, each labeled', async () => {
    stubFetch()
    const nodes: NodeInstance[] = [{ id: 'a', node_type: 'core.if', position: [0, 0], parameters: {}, disabled: false }]
    const wrapper = mount(WorkflowCanvas, { props: { nodes, connections: [] } })
    await flush()

    const sourceHandles = wrapper.findAll('.vue-flow__handle.source')
    // 2 declared ports ("true"/"false") + 1 universal error handle = 3.
    expect(sourceHandles.length).toBe(3)
    const ids = sourceHandles.map((h) => h.attributes('data-handleid')).sort()
    expect(ids).toEqual(['0', '1', 'error'])
    expect(wrapper.text()).toContain('true')
    expect(wrapper.text()).toContain('false')
    expect(wrapper.text()).toContain('error')
  })

  it('a single-port node still renders exactly one success handle plus the error handle', async () => {
    stubFetch(NODE_TYPES, ['main'])
    const nodes: NodeInstance[] = [{ id: 'a', node_type: 'core.manualTrigger', position: [0, 0], parameters: {}, disabled: false }]
    const wrapper = mount(WorkflowCanvas, { props: { nodes, connections: [] } })
    await flush()

    const sourceHandles = wrapper.findAll('.vue-flow__handle.source')
    expect(sourceHandles.length).toBe(2)
    const ids = sourceHandles.map((h) => h.attributes('data-handleid')).sort()
    expect(ids).toEqual(['0', 'error'])
  })

  it('mounts successfully with a pre-loaded error-routed connection', async () => {
    stubFetch(NODE_TYPES, ['true', 'false'])
    const nodes: NodeInstance[] = [
      { id: 'a', node_type: 'core.if', position: [0, 0], parameters: {}, disabled: false },
      { id: 'b', node_type: 'core.manualTrigger', position: [200, 0], parameters: {}, disabled: false },
    ]
    const wrapper = mount(WorkflowCanvas, { props: { nodes, connections: [] } })
    await flush()

    // Simulate VueFlow's onConnect callback directly via the exposed handler
    // is not accessible from outside; instead assert via a loaded connection
    // round-trip, which exercises the same mapping logic in flowEdges.
    const errorConnection: Connection = { from_node: 'a', from_output: 0, to_node: 'b', to_input: 0, error: true }
    await wrapper.setProps({ connections: [errorConnection] })
    await flush()
    // This test only confirms mounting with an error connection doesn't
    // throw and the component accepts the shape. The actual routing/mapping
    // logic (error connections attaching to the "error" handle with a red
    // stroke) is covered directly and deterministically by the
    // connectionFromVueFlow / edgeSourceHandle / edgeStyle unit tests below.
    expect(wrapper.exists()).toBe(true)
  })

  it('falls back to a single main port when portsFor fails', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn((url: string) => {
        if (url === '/rest/r8r/node-types') {
          return Promise.resolve({ ok: true, status: 200, json: async () => NODE_TYPES })
        }
        return Promise.reject(new Error('network error'))
      }),
    )
    const nodes: NodeInstance[] = [{ id: 'a', node_type: 'core.if', position: [0, 0], parameters: {}, disabled: false }]
    const wrapper = mount(WorkflowCanvas, { props: { nodes, connections: [] } })
    await flush()

    const sourceHandles = wrapper.findAll('.vue-flow__handle.source')
    // Falls back to 1 main port + 1 error handle = 2, not 2 declared + 1 = 3.
    expect(sourceHandles.length).toBe(2)
  })

  it("renders each node's icon and display name, with the raw type as a tooltip", async () => {
    stubFetch()
    const nodes: NodeInstance[] = [{ id: 'a', node_type: 'core.manualTrigger', position: [0, 0], parameters: {}, disabled: false }]
    const wrapper = mount(WorkflowCanvas, { props: { nodes, connections: [] } })
    await flush()
    // A trigger is a triangle: its icon inside, its name below.
    expect(wrapper.text()).toContain('🖱️')
    expect(wrapper.text()).toContain('Manual Trigger')
    expect(wrapper.find('[title="core.manualTrigger"]').exists()).toBe(true)
  })

  it('draws triggers as starts, nodes nothing follows as ends, the rest as boxes', async () => {
    stubFetch()
    const nodes: NodeInstance[] = [
      { id: 't', node_type: 'core.manualTrigger', position: [0, 0], parameters: {}, disabled: false },
      { id: 'mid', node_type: 'core.if', position: [200, 0], parameters: {}, disabled: false },
      { id: 'last', node_type: 'core.if', position: [400, 0], parameters: {}, disabled: false },
    ]
    const connections: Connection[] = [
      { from_node: 't', from_output: 0, to_node: 'mid', to_input: 0, error: false },
      { from_node: 'mid', from_output: 0, to_node: 'last', to_input: 0, error: false },
    ]
    const wrapper = mount(WorkflowCanvas, { props: { nodes, connections } })
    await flush()
    const shapes = wrapper.findAll('[data-shape]').map((w) => w.attributes('data-shape'))
    expect(shapes).toEqual(['start', 'box', 'end'])
    // Inputs come in on the left, outputs leave on the right; a start has no input.
    const targets = wrapper.findAll('.vue-flow__handle.target')
    expect(targets).toHaveLength(2)
    expect(targets.every((h) => h.classes().includes('vue-flow__handle-left'))).toBe(true)
    expect(wrapper.findAll('.vue-flow__handle.source').every((h) => h.classes().includes('vue-flow__handle-right'))).toBe(true)
  })

  it('marks each node with how it fared in the run on screen', async () => {
    stubFetch()
    const nodes: NodeInstance[] = [
      { id: 'a', node_type: 'core.manualTrigger', position: [0, 0], parameters: {}, disabled: false },
      { id: 'b', node_type: 'core.if', position: [200, 0], parameters: {}, disabled: false },
      { id: 'c', node_type: 'core.if', position: [400, 0], parameters: {}, disabled: false },
    ]
    const wrapper = mount(WorkflowCanvas, { props: { nodes, connections: [] } })
    await flush()
    expect(wrapper.findAll('[data-run]')).toHaveLength(0) // no run shown: no colours
    await wrapper.setProps({
      execution: {
        id: 'e', workflow_id: 'w', status: 'Running', mode: 'Manual', node_outputs: {}, started_at: '', finished_at: null,
        node_runs: { a: { status: 'success', counts: { '0': 1 } }, b: { status: 'running', counts: {} } },
      },
    })
    await flush()
    expect(wrapper.findAll('[data-run]').map((w) => w.attributes('data-run'))).toEqual(['success', 'running', 'pending'])
  })

  describe('AI Agent node', () => {
    const agentNode = (parameters: Record<string, unknown>): NodeInstance => ({ id: 'agent', node_type: 'ai.agent', position: [0, 0], parameters, disabled: false })
    const ready = { model: 'llama3.1', user_message: 'hi' }

    it('has chat model, memory and tool ports below it, chat model marked required', async () => {
      stubFetch(NODE_TYPES, ['main'])
      const wrapper = mount(WorkflowCanvas, { props: { nodes: [agentNode(ready)], connections: [] } })
      await flush()
      const ports = wrapper.findAll('[data-testid^="aux-port-"]')
      expect(ports.map((p) => p.attributes('data-testid'))).toEqual(['aux-port-model', 'aux-port-memory', 'aux-port-tools'])
      expect(ports.map((p) => p.find('[data-testid="aux-label"]').text())).toEqual(['chat model*', 'memory', 'tool'])
      expect(ports[0].find('[data-testid="aux-required"]').exists()).toBe(true)
      expect(ports[1].find('[data-testid="aux-required"]').exists()).toBe(false)
      // A box, even with nothing after it, so the ports have an edge to hang from.
      expect(wrapper.find('[data-shape]').attributes('data-shape')).toBe('box')
    })

    it("marks a port that's set up", async () => {
      stubFetch(NODE_TYPES, ['main'])
      const wrapper = mount(WorkflowCanvas, { props: { nodes: [agentNode({ ...ready, memory: { enabled: true } })], connections: [] } })
      await flush()
      expect(wrapper.find('[data-testid="aux-port-model"]').attributes('data-set')).toBe('true')
      expect(wrapper.find('[data-testid="aux-port-memory"]').attributes('data-set')).toBe('true')
      expect(wrapper.find('[data-testid="aux-port-tools"]').attributes('data-set')).toBe('false')
    })

    it("shows the chosen model's name under its port, which glows green", async () => {
      stubFetch(NODE_TYPES, ['main'])
      const wrapper = mount(WorkflowCanvas, { props: { nodes: [agentNode(ready)], connections: [] } })
      await flush()
      const port = wrapper.find('[data-testid="aux-port-model"]')
      expect(port.find('[data-testid="aux-detail"]').text()).toBe('llama3.1')
      expect(port.find('button').attributes('style')).toContain('box-shadow')
    })

    it('shows no model name or glow before a model is chosen', async () => {
      stubFetch(NODE_TYPES, ['main'])
      const wrapper = mount(WorkflowCanvas, { props: { nodes: [agentNode({ user_message: 'hi' })], connections: [] } })
      await flush()
      const port = wrapper.find('[data-testid="aux-port-model"]')
      expect(port.find('[data-testid="aux-detail"]').exists()).toBe(false)
      expect(port.find('button').attributes('style') ?? '').not.toContain('box-shadow')
    })

    it('opens the matching settings from a port', async () => {
      stubFetch(NODE_TYPES, ['main'])
      const wrapper = mount(WorkflowCanvas, { props: { nodes: [agentNode(ready)], connections: [] } })
      await flush()
      await wrapper.find('[data-testid="aux-port-memory"] button').trigger('click')
      expect(wrapper.emitted('aux-open')).toEqual([['agent', 'memory']])
    })

    it('shows the red "!" when the agent is set up but its last run failed', async () => {
      stubFetch(NODE_TYPES, ['main'])
      const wrapper = mount(WorkflowCanvas, { props: { nodes: [agentNode(ready)], connections: [] } })
      await flush()
      expect(wrapper.find('[data-testid="needs-setup"]').exists()).toBe(false)
      await wrapper.setProps({
        execution: { id: 'e', workflow_id: 'w', status: 'Error', mode: 'Manual', started_at: '', finished_at: null,
          node_outputs: { agent: [{ json: { error: 'model not found' }, binary: {} }] }, node_runs: { agent: { status: 'error', counts: {} } } },
      })
      await flush()
      expect(wrapper.find('[data-testid="needs-setup"]').attributes('title')).toContain('model not found')
    })
  })

  it("hangs an AI Agent's tools below it", async () => {
    stubFetch(NODE_TYPES, ['main'], [{ id: 't1', name: 'Weather' }, { id: 't2', name: 'Search' }])
    const nodes: NodeInstance[] = [
      { id: 'agent', node_type: 'ai.agent', position: [0, 0], parameters: { model: 'm', user_message: 'u', tool_ids: ['t1', 't2'] }, disabled: false },
    ]
    const wrapper = mount(WorkflowCanvas, { props: { nodes, connections: [] } })
    await flush()
    expect(wrapper.find('[data-testid="agent-tools"]').text()).toBe('🔧 Weather🔧 Search')
  })

  it('marks an AI Agent that is missing its model or message as needing setup', async () => {
    stubFetch()
    const nodes: NodeInstance[] = [
      { id: 'bare', node_type: 'ai.agent', position: [0, 0], parameters: {}, disabled: false },
      { id: 'ready', node_type: 'ai.agent', position: [0, 200], parameters: { model: 'qwen', user_message: 'hi' }, disabled: false },
    ]
    const wrapper = mount(WorkflowCanvas, { props: { nodes, connections: [] } })
    await flush()
    const badges = wrapper.findAll('[data-testid="needs-setup"]')
    expect(badges).toHaveLength(1)
    expect(badges[0].text()).toBe('!')
    expect(badges[0].attributes('title')).toContain('Model')
  })

  it('falls back to the raw type_name for an unknown node type', async () => {
    stubFetch()
    const nodes: NodeInstance[] = [{ id: 'a', node_type: 'does.not.exist', position: [0, 0], parameters: {}, disabled: false }]
    const wrapper = mount(WorkflowCanvas, { props: { nodes, connections: [] } })
    await flush()
    expect(wrapper.text()).toContain('does.not.exist')
  })

  it('a dynamic-port-count node (core.switch) renders one handle per resolved port', async () => {
    stubFetch(NODE_TYPES, ['case 0', 'case 1', 'case 2', 'default'])
    const nodes: NodeInstance[] = [{ id: 'a', node_type: 'core.if', position: [0, 0], parameters: { cases: ['x', 'y', 'z'] }, disabled: false }]
    const wrapper = mount(WorkflowCanvas, { props: { nodes, connections: [] } })
    await flush()
    const sourceHandles = wrapper.findAll('.vue-flow__handle.source')
    // 4 declared ports + 1 universal error handle = 5.
    expect(sourceHandles.length).toBe(5)
    expect(wrapper.text()).toContain('case 0')
    expect(wrapper.text()).toContain('default')
  })
})

describe('connectionFromVueFlow', () => {
  it('maps a normal numbered source handle to error: false', () => {
    const result = connectionFromVueFlow({ source: 'a', sourceHandle: '1', target: 'b', targetHandle: '0' })
    expect(result).toEqual({ from_node: 'a', from_output: 1, to_node: 'b', to_input: 0, error: false })
  })

  it('maps the error handle to error: true, from_output: 0', () => {
    const result = connectionFromVueFlow({ source: 'a', sourceHandle: 'error', target: 'b', targetHandle: '0' })
    expect(result).toEqual({ from_node: 'a', from_output: 0, to_node: 'b', to_input: 0, error: true })
  })
})

describe('edgeSourceHandle / edgeStyle', () => {
  it('a normal connection maps to its numbered handle with no special style', () => {
    const c: Connection = { from_node: 'a', from_output: 2, to_node: 'b', to_input: 0, error: false }
    expect(edgeSourceHandle(c)).toBe('2')
    expect(edgeStyle(c)).toBeUndefined()
  })

  it('an error-routed connection maps to the error handle with a red stroke style', () => {
    const c: Connection = { from_node: 'a', from_output: 0, to_node: 'b', to_input: 0, error: true }
    expect(edgeSourceHandle(c)).toBe('error')
    expect(edgeStyle(c)).toEqual({ stroke: '#dc2626' })
  })
})
