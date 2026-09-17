import { describe, it, expect, beforeAll } from 'vitest'
import { mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import WorkflowCanvas from './WorkflowCanvas.vue'
import type { NodeInstance } from '../types/domain'

// jsdom does not implement ResizeObserver, which @vue-flow/core relies on to
// measure node dimensions. Vue Flow only needs the constructor to exist and
// observe/unobserve to be callable; it doesn't depend on entries firing for
// nodes to render with their labels.
beforeAll(() => {
  class ResizeObserverStub {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  ;(globalThis as unknown as { ResizeObserver: typeof ResizeObserverStub }).ResizeObserver = ResizeObserverStub
})

describe('WorkflowCanvas', () => {
  it('renders one canvas node per workflow node', async () => {
    const nodes: NodeInstance[] = [
      { id: 'a', node_type: 'core.manualTrigger', position: [0, 0], parameters: {}, disabled: false },
      { id: 'b', node_type: 'core.set', position: [200, 0], parameters: {}, disabled: false },
    ]
    const wrapper = mount(WorkflowCanvas, { props: { nodes, connections: [] } })
    // Vue Flow creates its ResizeObserver in onMounted and only then renders
    // its node list, so wait a tick for that post-mount update to flush.
    await nextTick()
    expect(wrapper.text()).toContain('core.manualTrigger')
    expect(wrapper.text()).toContain('core.set')
  })
})
