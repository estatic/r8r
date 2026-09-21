import { describe, it, expect, beforeEach, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import AddNodeMenu from './AddNodeMenu.vue'

const NODE_TYPES = [
  { type_name: 'core.set', display_name: 'Set', icon: '📝', category: 'action', description: 'Adds or overwrites fields on each item.', credential_types: [], output_ports: ['main'] },
  { type_name: 'core.httpRequest', display_name: 'HTTP Request', icon: '🌐', category: 'action', description: 'Makes an HTTP request to an external URL.', credential_types: [], output_ports: ['main'] },
]

describe('AddNodeMenu', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        status: 200,
        json: async () => NODE_TYPES,
      }),
    )
  })

  it('shows icon, display name, and description, and emits add with the type_name when clicked', async () => {
    const wrapper = mount(AddNodeMenu)
    await wrapper.find('button').trigger('click')
    await new Promise((r) => setTimeout(r, 0)) // let fetchAll's promise resolve
    expect(wrapper.text()).toContain('🌐 HTTP Request')
    expect(wrapper.text()).toContain('Makes an HTTP request to an external URL.')
    const items = wrapper.findAll('li')
    const httpItem = items.find((li) => li.text().includes('HTTP Request'))
    expect(httpItem).toBeTruthy()
    await httpItem!.trigger('click')
    expect(wrapper.emitted('add')).toEqual([['core.httpRequest']])
    vi.unstubAllGlobals()
  })

  it('search matches against display name as well as type_name', async () => {
    const wrapper = mount(AddNodeMenu)
    await wrapper.find('button').trigger('click')
    await new Promise((r) => setTimeout(r, 0))
    await wrapper.find('input').setValue('http')
    expect(wrapper.text()).toContain('HTTP Request')
    expect(wrapper.text()).not.toContain('📝 Set')
    vi.unstubAllGlobals()
  })
})
