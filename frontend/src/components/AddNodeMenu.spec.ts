import { describe, it, expect, beforeEach, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import AddNodeMenu from './AddNodeMenu.vue'

describe('AddNodeMenu', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        status: 200,
        json: async () => ['core.set', 'core.httpRequest', 'telegram.sendMessage'],
      }),
    )
  })

  it('emits add with the chosen type when clicked', async () => {
    const wrapper = mount(AddNodeMenu)
    await wrapper.find('button').trigger('click')
    await new Promise((r) => setTimeout(r, 0)) // let fetchAll's promise resolve
    const items = wrapper.findAll('li')
    const httpItem = items.find((li) => li.text() === 'core.httpRequest')
    expect(httpItem).toBeTruthy()
    await httpItem!.trigger('click')
    expect(wrapper.emitted('add')).toEqual([['core.httpRequest']])
    vi.unstubAllGlobals()
  })
})
