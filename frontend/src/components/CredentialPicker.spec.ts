import { describe, it, expect, beforeEach, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import CredentialPicker from './CredentialPicker.vue'

function stubFetch(nodeTypes: unknown[], credentials: unknown[] = []) {
  vi.stubGlobal(
    'fetch',
    vi.fn((url: string) => {
      if (url === '/rest/node-types') {
        return Promise.resolve({ ok: true, status: 200, json: async () => nodeTypes })
      }
      if (url === '/rest/credentials') {
        return Promise.resolve({ ok: true, status: 200, json: async () => credentials })
      }
      return Promise.reject(new Error(`unexpected fetch: ${url}`))
    }),
  )
}

describe('CredentialPicker', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it("shows a type dropdown restricted to the node's declared credential types", async () => {
    stubFetch([
      {
        type_name: 'telegram.trigger',
        display_name: 'Telegram Trigger',
        icon: '📨',
        category: 'trigger',
        description: '',
        credential_types: ['telegramApi'],
        output_ports: ['main'],
      },
    ])
    const wrapper = mount(CredentialPicker, { props: { modelValue: null, nodeType: 'telegram.trigger' } })
    await wrapper.find('button').trigger('click')
    await new Promise((r) => setTimeout(r, 0))
    const selects = wrapper.findAll('select')
    // selects[0] is the existing-credentials picker; the new "+ New credential"
    // form's Type field is a second select only when credential_types is non-empty.
    expect(selects.length).toBe(2)
    expect(selects[1].findAll('option').map((o) => o.text())).toEqual(['Select a type…', 'telegramApi'])
    vi.unstubAllGlobals()
  })

  it('falls back to a free-text type field when the node declares no credential types', async () => {
    stubFetch([
      {
        type_name: 'core.httpRequest',
        display_name: 'HTTP Request',
        icon: '🌐',
        category: 'action',
        description: '',
        credential_types: [],
        output_ports: ['main'],
      },
    ])
    const wrapper = mount(CredentialPicker, { props: { modelValue: null, nodeType: 'core.httpRequest' } })
    await wrapper.find('button').trigger('click')
    await new Promise((r) => setTimeout(r, 0))
    expect(wrapper.findAll('select').length).toBe(1) // only the existing-credentials picker
    expect(wrapper.find('input[placeholder^="Type"]').exists()).toBe(true)
    vi.unstubAllGlobals()
  })
})
