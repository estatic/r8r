import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import NodeConfigPanel from './NodeConfigPanel.vue'
import type { NodeInstance, NodeSettings } from '../types/domain'

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

  async function clickApply(wrapper: ReturnType<typeof mount>) {
    const buttons = wrapper.findAll('button')
    await buttons[buttons.length - 1].trigger('click')
  }

  function emittedSettings(wrapper: ReturnType<typeof mount>): NodeSettings | undefined {
    const events = wrapper.emitted('update')
    return events ? (events[0][0] as NodeInstance).settings : undefined
  }

  it('loads a node without settings with retry off and default retry values', async () => {
    const wrapper = mount(NodeConfigPanel, { props: { node } })
    expect((wrapper.find('[data-testid="retry-enabled"]').element as HTMLInputElement).checked).toBe(false)
    expect(wrapper.find('[data-testid="max-tries"]').exists()).toBe(false)
    await wrapper.find('[data-testid="retry-enabled"]').setValue(true)
    expect((wrapper.find('[data-testid="max-tries"]').element as HTMLInputElement).value).toBe('3')
    expect((wrapper.find('[data-testid="wait-ms"]').element as HTMLInputElement).value).toBe('1000')
  })

  it('emits default settings when nothing is changed', async () => {
    const wrapper = mount(NodeConfigPanel, { props: { node } })
    await clickApply(wrapper)
    expect(emittedSettings(wrapper)).toEqual({ retry: null, timeout_ms: null, continue_on_fail: false })
  })

  it('emits the configured settings on Apply', async () => {
    const wrapper = mount(NodeConfigPanel, { props: { node } })
    await wrapper.find('[data-testid="continue-on-fail"]').setValue(true)
    await wrapper.find('[data-testid="retry-enabled"]').setValue(true)
    await wrapper.find('[data-testid="max-tries"]').setValue('5')
    await wrapper.find('[data-testid="wait-ms"]').setValue('250')
    await wrapper.find('[data-testid="timeout-ms"]').setValue('3000')
    await clickApply(wrapper)
    expect(emittedSettings(wrapper)).toEqual({ retry: { max_tries: 5, wait_ms: 250 }, timeout_ms: 3000, continue_on_fail: true })
  })

  it('emits timeout_ms null when the timeout field is cleared', async () => {
    const withTimeout: NodeInstance = { ...node, settings: { retry: null, timeout_ms: 500, continue_on_fail: false } }
    const wrapper = mount(NodeConfigPanel, { props: { node: withTimeout } })
    await wrapper.find('[data-testid="timeout-ms"]').setValue('')
    await clickApply(wrapper)
    expect(emittedSettings(wrapper)?.timeout_ms).toBeNull()
  })

  it('preserves a stored wait of 0 ms', async () => {
    const zeroWait: NodeInstance = { ...node, settings: { retry: { max_tries: 4, wait_ms: 0 }, timeout_ms: null, continue_on_fail: false } }
    const wrapper = mount(NodeConfigPanel, { props: { node: zeroWait } })
    await clickApply(wrapper)
    expect(emittedSettings(wrapper)?.retry).toEqual({ max_tries: 4, wait_ms: 0 })
  })

  it('rejects a cleared wait field instead of silently saving 0 ms', async () => {
    const wrapper = mount(NodeConfigPanel, { props: { node } })
    await wrapper.find('[data-testid="retry-enabled"]').setValue(true)
    await wrapper.find('[data-testid="wait-ms"]').setValue('')
    await clickApply(wrapper)
    expect(wrapper.text()).toContain('Wait between tries must be between 0 and 60000 ms.')
    expect(wrapper.emitted('update')).toBeFalsy()
  })

  it('shows an error and emits nothing for out-of-range max tries', async () => {
    const wrapper = mount(NodeConfigPanel, { props: { node } })
    await wrapper.find('[data-testid="retry-enabled"]').setValue(true)
    await wrapper.find('[data-testid="max-tries"]').setValue('11')
    await clickApply(wrapper)
    expect(wrapper.text()).toContain('Max tries must be between 2 and 10.')
    expect(wrapper.emitted('update')).toBeFalsy()
  })
})
