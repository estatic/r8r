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

  it('renders a source and a target connection handle on every node', async () => {
    const nodes: NodeInstance[] = [
      { id: 'a', node_type: 'core.manualTrigger', position: [0, 0], parameters: {}, disabled: false },
      { id: 'b', node_type: 'core.set', position: [200, 0], parameters: {}, disabled: false },
    ]
    const wrapper = mount(WorkflowCanvas, { props: { nodes, connections: [] } })
    await nextTick()

    // Our #node-default slot replaces Vue Flow's DefaultNode, which is what
    // would otherwise render the handles a user drags a connection from. Two
    // handles per node (one target, one source) must exist in the DOM, or
    // connecting nodes is impossible in a real browser.
    const handles = wrapper.findAll('.vue-flow__handle')
    expect(handles.length).toBe(4)
    expect(wrapper.findAll('.vue-flow__handle.target').length).toBe(2)
    expect(wrapper.findAll('.vue-flow__handle.source').length).toBe(2)
    // Handle ids must match the `sourceHandle`/`targetHandle` strings that
    // flowEdges derives from from_output/to_input, so existing connections
    // attach to the handle elements themselves.
    expect(handles.every((h) => h.attributes('data-handleid') === '0')).toBe(true)
  })
})
