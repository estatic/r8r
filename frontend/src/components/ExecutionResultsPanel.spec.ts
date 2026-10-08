import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import ExecutionResultsPanel from './ExecutionResultsPanel.vue'
import type { Execution } from '../types/domain'

const execution: Execution = {
  id: 'e1',
  workflow_id: 'w1',
  status: 'Success',
  mode: 'Manual',
  node_outputs: { n1: [{ json: { hello: 'world' }, binary: {} }] },
  started_at: 'x',
  finished_at: 'y',
}

describe('ExecutionResultsPanel', () => {
  it('renders the status and each node\'s output', () => {
    const wrapper = mount(ExecutionResultsPanel, { props: { execution } })
    expect(wrapper.text()).toContain('Success')
    expect(wrapper.text()).toContain('n1')
    expect(wrapper.text()).toContain('hello')
  })

  it("shows a failed node's error as readable text, line by line, not as JSON", () => {
    const error = "node code failed: TypeError: cannot read property 'deep' of undefined\n  at f (line 3:20)    return x.missing.deep;\n  at main code (line 5:9)    return [f({})];"
    const failed: Execution = {
      ...execution,
      status: 'Error',
      node_outputs: { code: [{ json: { error }, binary: {} }] },
      node_runs: { code: { status: 'error', counts: {} } },
    }
    const wrapper = mount(ExecutionResultsPanel, { props: { execution: failed } })
    const shown = wrapper.find('[data-testid="node-error"]')
    expect(shown.text()).toBe(error)
    expect(shown.text()).not.toContain('\\n')
    expect(shown.classes()).toContain('whitespace-pre-wrap')
  })

  it('renders nothing when there is no execution', () => {
    const wrapper = mount(ExecutionResultsPanel, { props: { execution: null } })
    expect(wrapper.text()).toBe('')
  })

  it('renders no history selector when history is empty', () => {
    const wrapper = mount(ExecutionResultsPanel, { props: { execution, history: [] } })
    expect(wrapper.find('select').exists()).toBe(false)
  })

  it('offers past runs in a selector and emits select on change', async () => {
    const older: Execution = { ...execution, id: 'e0', started_at: 'earlier' }
    const wrapper = mount(ExecutionResultsPanel, { props: { execution, history: [execution, older] } })

    const select = wrapper.find('select')
    expect(select.exists()).toBe(true)
    await select.setValue('e0')

    expect(wrapper.emitted('select')?.[0]).toEqual([older])
  })
})
