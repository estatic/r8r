import { describe, it, expect, beforeAll, beforeEach, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import { createPinia, setActivePinia } from 'pinia'
import WorkflowCanvas from './WorkflowCanvas.vue'
import type { NodeInstance } from '../types/domain'

beforeAll(() => {
  class ResizeObserverStub {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  ;(globalThis as unknown as { ResizeObserver: typeof ResizeObserverStub }).ResizeObserver = ResizeObserverStub
})

const NODE_TYPES = [
  { type_name: 'core.manualTrigger', display_name: 'Manual Trigger', icon: '🖱️', category: 'trigger', description: '', credential_types: [], output_ports: ['main'] },
  { type_name: 'core.set', display_name: 'Set', icon: '📝', category: 'action', description: '', credential_types: [], output_ports: ['main'] },
]

async function flushFetch() {
  await new Promise((r) => setTimeout(r, 0))
  await nextTick()
}

describe('WorkflowCanvas', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({ ok: true, status: 200, json: async () => NODE_TYPES }),
    )
  })

  it("renders each node's icon and display name, with the raw type as a tooltip", async () => {
    const nodes: NodeInstance[] = [
      { id: 'a', node_type: 'core.manualTrigger', position: [0, 0], parameters: {}, disabled: false },
      { id: 'b', node_type: 'core.set', position: [200, 0], parameters: {}, disabled: false },
    ]
    const wrapper = mount(WorkflowCanvas, { props: { nodes, connections: [] } })
    await nextTick()
    await flushFetch()
    expect(wrapper.text()).toContain('🖱️ Manual Trigger')
    expect(wrapper.text()).toContain('📝 Set')
    expect(wrapper.find('[title="core.manualTrigger"]').exists()).toBe(true)
  })

  it('falls back to the raw type_name for an unknown node type', async () => {
    const nodes: NodeInstance[] = [{ id: 'a', node_type: 'does.not.exist', position: [0, 0], parameters: {}, disabled: false }]
    const wrapper = mount(WorkflowCanvas, { props: { nodes, connections: [] } })
    await nextTick()
    await flushFetch()
    expect(wrapper.text()).toContain('does.not.exist')
  })

  it('renders a source and a target connection handle on every node', async () => {
    const nodes: NodeInstance[] = [
      { id: 'a', node_type: 'core.manualTrigger', position: [0, 0], parameters: {}, disabled: false },
      { id: 'b', node_type: 'core.set', position: [200, 0], parameters: {}, disabled: false },
    ]
    const wrapper = mount(WorkflowCanvas, { props: { nodes, connections: [] } })
    await nextTick()

    const handles = wrapper.findAll('.vue-flow__handle')
    expect(handles.length).toBe(4)
    expect(wrapper.findAll('.vue-flow__handle.target').length).toBe(2)
    expect(wrapper.findAll('.vue-flow__handle.source').length).toBe(2)
    expect(handles.every((h) => h.attributes('data-handleid') === '0')).toBe(true)
  })
})
