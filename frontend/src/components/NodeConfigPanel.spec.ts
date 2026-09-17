import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import NodeConfigPanel from './NodeConfigPanel.vue'
import type { NodeInstance } from '../types/domain'

const node: NodeInstance = {
  id: 'n1',
  node_type: 'core.set',
  position: [0, 0],
  parameters: { foo: 'bar' },
  disabled: false,
}

describe('NodeConfigPanel', () => {
  it('emits update with the parsed parameters on Apply', async () => {
    const wrapper = mount(NodeConfigPanel, { props: { node } })
    const textarea = wrapper.find('textarea')
    await textarea.setValue('{"foo":"baz"}')
    const buttons = wrapper.findAll('button')
    await buttons[buttons.length - 1].trigger('click')
    const events = wrapper.emitted('update')
    expect(events).toBeTruthy()
    expect((events![0][0] as NodeInstance).parameters).toEqual({ foo: 'baz' })
  })

  it('shows an error and does not emit update on invalid JSON', async () => {
    const wrapper = mount(NodeConfigPanel, { props: { node } })
    const textarea = wrapper.find('textarea')
    await textarea.setValue('{not valid json')
    const buttons = wrapper.findAll('button')
    await buttons[buttons.length - 1].trigger('click')
    expect(wrapper.text()).toContain('must be valid JSON')
    expect(wrapper.emitted('update')).toBeFalsy()
  })
})
