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
