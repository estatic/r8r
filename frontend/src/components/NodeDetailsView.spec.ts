import { describe, expect, it, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import NodeDetailsView from './NodeDetailsView.vue'
import { useNodeTypesStore } from '../stores/nodeTypes'
import type { Execution, NodeInstance, Workflow } from '../types/domain'

const item = (json: Record<string, unknown>) => ({ json, binary: {} })
const node = (id: string, node_type: string): NodeInstance => ({ id, node_type, position: [0, 0], parameters: {}, disabled: false })
const workflow = {
  id: 'w',
  name: 'w',
  active: false,
  nodes: [node('t', 'core.manualTrigger'), node('if1', 'core.if'), node('code1', 'core.code')],
  connections: [
    { from_node: 't', from_output: 0, to_node: 'if1', to_input: 0, error: false },
    { from_node: 'if1', from_output: 0, to_node: 'code1', to_input: 0, error: false },
  ],
} as unknown as Workflow
const execution: Execution = {
  id: 'e',
  workflow_id: 'w',
  status: 'Error',
  mode: 'Manual',
  node_outputs: {},
  node_runs: {
    t: { status: 'success', counts: { '0': 1 }, outputs: [[item({ n: 5 })]], reused: true },
    if1: { status: 'success', counts: { '0': 1, '1': 0 }, input: [item({ n: 5 })], outputs: [[item({ n: 5 })], []] },
    code1: { status: 'error', counts: {}, input: [item({ n: 5 })], error: "TypeError: x is undefined\n  at main (line 2:9)    return x.y;" },
  },
  started_at: '',
  finished_at: null,
}

describe('NodeDetailsView', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    const types = useNodeTypesStore()
    types.types = [{ type_name: 'core.if', display_name: 'If', icon: '', category: 'flowControl', description: '', credential_types: [], output_ports: ['true', 'false'] }]
    types.loaded = true
  })

  it('shows what came in and each output, named by the node type', async () => {
    const w = mount(NodeDetailsView, { props: { node: node('if1', 'core.if'), workflow, execution } })
    expect(w.find('[data-testid="pane-input"] [data-testid="items-table"]').text()).toContain('5')
    const output = w.find('[data-testid="pane-output"]')
    expect(output.find('[data-testid="port-true"]').text()).toBe('true (1)')
    expect(output.find('[data-testid="port-false"]').text()).toBe('false (0)')
    await output.find('[data-testid="view-json"]').trigger('click')
    expect(output.find('[data-testid="items-json"]').text()).toContain('"n": 5')
  })

  it('shows a failed node\'s error with its stack trace instead of the output', () => {
    const w = mount(NodeDetailsView, { props: { node: node('code1', 'core.code'), workflow, execution } })
    const err = w.find('[data-testid="pane-output"] [data-testid="node-error-details"]')
    expect(err.text()).toContain('TypeError: x is undefined')
    expect(err.text()).toContain('at main (line 2:9)')
    expect(w.find('[data-testid="pane-input"]').text()).toContain('1 item')
  })

  it('says a trigger has no input, marks a reused node, and invites a run when there is none', () => {
    const trig = mount(NodeDetailsView, { props: { node: node('t', 'core.manualTrigger'), workflow, execution } })
    expect(trig.find('[data-testid="pane-input"]').text()).toContain('has no input')
    expect(trig.find('[data-testid="pane-output"]').text()).toContain('reused from the last run')
    const none = mount(NodeDetailsView, { props: { node: node('if1', 'core.if'), workflow, execution: null } })
    expect(none.find('[data-testid="pane-output"]').text()).toContain('Execute the workflow')
  })

  it('closes when the backdrop is clicked', async () => {
    const w = mount(NodeDetailsView, { props: { node: node('if1', 'core.if'), workflow, execution } })
    await w.find('[data-testid="node-details"]').trigger('click')
    expect(w.emitted('close')).toBeTruthy()
  })
})
