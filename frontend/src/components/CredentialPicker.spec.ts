import { describe, it, expect, beforeEach, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import CredentialPicker from './CredentialPicker.vue'

const CREDENTIAL_TYPE_SCHEMAS = [
  { credential_type: 'telegramApi', display_name: 'Telegram Bot', generic: false, fields: [{ name: 'bot_token', label: 'Bot Token', field_type: 'password', required: true }] },
  { credential_type: 'bearerToken', display_name: 'Bearer Token', generic: true, fields: [{ name: 'token', label: 'Token', field_type: 'password', required: true }] },
  { credential_type: 'apiKeyHeader', display_name: 'API Key (Header)', generic: true, fields: [{ name: 'header_name', label: 'Header Name', field_type: 'text', required: true }, { name: 'value', label: 'Value', field_type: 'password', required: true }] },
  { credential_type: 'basicAuth', display_name: 'Basic Auth', generic: true, fields: [{ name: 'username', label: 'Username', field_type: 'text', required: true }, { name: 'password', label: 'Password (optional)', field_type: 'password', required: false }] },
]

// Shaped like the real ai.agent node: two restricted, non-generic types whose
// schemas share a field name (api_key) but not all fields (only openaiApi
// declares base_url). This is what makes the stale-key leak concretely
// reachable via the restricted `<select v-model="newType">` path.
const AI_AGENT_CREDENTIAL_TYPES = [
  { credential_type: 'anthropicApi', display_name: 'Anthropic API', generic: false, fields: [{ name: 'api_key', label: 'API Key', field_type: 'password', required: true }] },
  { credential_type: 'openaiApi', display_name: 'OpenAI API', generic: false, fields: [{ name: 'api_key', label: 'API Key', field_type: 'password', required: true }, { name: 'base_url', label: 'Base URL (optional)', field_type: 'text', required: false }] },
]

function stubFetch(nodeTypes: unknown[], credentials: unknown[] = [], credentialTypes: unknown[] = CREDENTIAL_TYPE_SCHEMAS) {
  vi.stubGlobal(
    'fetch',
    vi.fn((url: string) => {
      if (url === '/rest/node-types') return Promise.resolve({ ok: true, status: 200, json: async () => nodeTypes })
      if (url === '/rest/credentials') return Promise.resolve({ ok: true, status: 200, json: async () => credentials })
      if (url === '/rest/credential-types') return Promise.resolve({ ok: true, status: 200, json: async () => credentialTypes })
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

  it('renders a structured field for a node-restricted type with a known schema', async () => {
    stubFetch([
      { type_name: 'telegram.trigger', display_name: 'Telegram Trigger', icon: '📨', category: 'trigger', description: '', credential_types: ['telegramApi'], output_ports: ['main'] },
    ])
    const wrapper = mount(CredentialPicker, { props: { modelValue: null, nodeType: 'telegram.trigger' } })
    await wrapper.find('button').trigger('click')
    await new Promise((r) => setTimeout(r, 0))
    // Single accepted type auto-selects (existing toggleCreating behavior), so the
    // structured field should already be showing without an extra selection step.
    expect(wrapper.find('input[placeholder="Bot Token"]').exists()).toBe(true)
    expect(wrapper.find('input[placeholder="Bot Token"]').attributes('type')).toBe('password')
    expect(wrapper.find('textarea').exists()).toBe(false)
    vi.unstubAllGlobals()
  })

  it('offers generic types plus Custom for a node with no declared credential_types', async () => {
    stubFetch([
      { type_name: 'core.httpRequest', display_name: 'HTTP Request', icon: '🌐', category: 'action', description: '', credential_types: [], output_ports: ['main'] },
    ])
    const wrapper = mount(CredentialPicker, { props: { modelValue: null, nodeType: 'core.httpRequest' } })
    await wrapper.find('button').trigger('click')
    await new Promise((r) => setTimeout(r, 0))
    const options = wrapper.findAll('select')[1].findAll('option').map((o) => o.text())
    expect(options).toEqual(['Select a type…', 'Bearer Token', 'API Key (Header)', 'Basic Auth', 'Custom…'])
    vi.unstubAllGlobals()
  })

  it('selecting Custom reveals the free-text type field and raw JSON data field', async () => {
    stubFetch([
      { type_name: 'core.httpRequest', display_name: 'HTTP Request', icon: '🌐', category: 'action', description: '', credential_types: [], output_ports: ['main'] },
    ])
    const wrapper = mount(CredentialPicker, { props: { modelValue: null, nodeType: 'core.httpRequest' } })
    await wrapper.find('button').trigger('click')
    await new Promise((r) => setTimeout(r, 0))
    await wrapper.findAll('select')[1].setValue('__custom__')
    expect(wrapper.find('input[placeholder^="Type"]').exists()).toBe(true)
    expect(wrapper.find('textarea').exists()).toBe(true)
    vi.unstubAllGlobals()
  })

  it('submitting a structured form assembles the field values into the data object', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn((url: string, options?: RequestInit) => {
        if (url === '/rest/node-types') return Promise.resolve({ ok: true, status: 200, json: async () => [{ type_name: 'telegram.trigger', display_name: 'Telegram Trigger', icon: '📨', category: 'trigger', description: '', credential_types: ['telegramApi'], output_ports: ['main'] }] })
        if (url === '/rest/credentials' && (!options || options.method === undefined)) return Promise.resolve({ ok: true, status: 200, json: async () => [] })
        if (url === '/rest/credential-types') return Promise.resolve({ ok: true, status: 200, json: async () => CREDENTIAL_TYPE_SCHEMAS })
        if (url === '/rest/credentials' && options?.method === 'POST') {
          const body = JSON.parse(options.body as string)
          expect(body.data).toEqual({ bot_token: 'secret-token-value' })
          return Promise.resolve({ ok: true, status: 201, json: async () => ({ id: 'new-id', name: body.name, credential_type: body.credential_type, owner_id: 'u', created_at: '', updated_at: '' }) })
        }
        return Promise.reject(new Error(`unexpected fetch: ${url}`))
      }),
    )
    const wrapper = mount(CredentialPicker, { props: { modelValue: null, nodeType: 'telegram.trigger' } })
    await wrapper.find('button').trigger('click')
    await new Promise((r) => setTimeout(r, 0))
    await wrapper.find('input[placeholder="Name"]').setValue('My Bot')
    await wrapper.find('input[placeholder="Bot Token"]').setValue('secret-token-value')
    await wrapper.find('button.bg-blue-600').trigger('click')
    await new Promise((r) => setTimeout(r, 0))
    expect(wrapper.emitted('update:modelValue')).toEqual([['new-id']])
    vi.unstubAllGlobals()
  })

  it('switching the restricted type select clears stale field values so submitted data never contains a key outside the new schema', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn((url: string, options?: RequestInit) => {
        if (url === '/rest/node-types') return Promise.resolve({ ok: true, status: 200, json: async () => [{ type_name: 'ai.agent', display_name: 'AI Agent', icon: '🤖', category: 'action', description: '', credential_types: ['openaiApi', 'anthropicApi'], output_ports: ['main'] }] })
        if (url === '/rest/credentials' && (!options || options.method === undefined)) return Promise.resolve({ ok: true, status: 200, json: async () => [] })
        if (url === '/rest/credential-types') return Promise.resolve({ ok: true, status: 200, json: async () => AI_AGENT_CREDENTIAL_TYPES })
        if (url === '/rest/credentials' && options?.method === 'POST') {
          const body = JSON.parse(options.body as string)
          expect(body.data).toEqual({ api_key: 'anthropic-key' })
          return Promise.resolve({ ok: true, status: 201, json: async () => ({ id: 'new-id', name: body.name, credential_type: body.credential_type, owner_id: 'u', created_at: '', updated_at: '' }) })
        }
        return Promise.reject(new Error(`unexpected fetch: ${url}`))
      }),
    )
    const wrapper = mount(CredentialPicker, { props: { modelValue: null, nodeType: 'ai.agent' } })
    await wrapper.find('button').trigger('click')
    await new Promise((r) => setTimeout(r, 0))
    // Two accepted types, so nothing auto-selects: pick openaiApi first.
    const typeSelect = wrapper.findAll('select')[1]
    await typeSelect.setValue('openaiApi')
    await wrapper.find('input[placeholder="API Key"]').setValue('openai-key')
    await wrapper.find('input[placeholder="Base URL (optional)"]').setValue('https://example.com')
    // Now switch to anthropicApi, which has no base_url field at all.
    await typeSelect.setValue('anthropicApi')
    await wrapper.find('input[placeholder="Name"]').setValue('My Agent Cred')
    await wrapper.find('input[placeholder="API Key"]').setValue('anthropic-key')
    await wrapper.find('button.bg-blue-600').trigger('click')
    await new Promise((r) => setTimeout(r, 0))
    expect(wrapper.emitted('update:modelValue')).toEqual([['new-id']])
    vi.unstubAllGlobals()
  })

  it('omits a cleared optional field from the submitted data instead of sending an empty string', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn((url: string, options?: RequestInit) => {
        if (url === '/rest/node-types') return Promise.resolve({ ok: true, status: 200, json: async () => [{ type_name: 'ai.agent', display_name: 'AI Agent', icon: '🤖', category: 'action', description: '', credential_types: ['openaiApi'], output_ports: ['main'] }] })
        if (url === '/rest/credentials' && (!options || options.method === undefined)) return Promise.resolve({ ok: true, status: 200, json: async () => [] })
        if (url === '/rest/credential-types') return Promise.resolve({ ok: true, status: 200, json: async () => AI_AGENT_CREDENTIAL_TYPES })
        if (url === '/rest/credentials' && options?.method === 'POST') {
          const body = JSON.parse(options.body as string)
          expect(body.data).toEqual({ api_key: 'openai-key' })
          expect(Object.prototype.hasOwnProperty.call(body.data, 'base_url')).toBe(false)
          return Promise.resolve({ ok: true, status: 201, json: async () => ({ id: 'new-id', name: body.name, credential_type: body.credential_type, owner_id: 'u', created_at: '', updated_at: '' }) })
        }
        return Promise.reject(new Error(`unexpected fetch: ${url}`))
      }),
    )
    const wrapper = mount(CredentialPicker, { props: { modelValue: null, nodeType: 'ai.agent' } })
    await wrapper.find('button').trigger('click')
    await new Promise((r) => setTimeout(r, 0))
    // Single accepted type auto-selects openaiApi.
    await wrapper.find('input[placeholder="Name"]').setValue('My OpenAI Cred')
    await wrapper.find('input[placeholder="API Key"]').setValue('openai-key')
    await wrapper.find('input[placeholder="Base URL (optional)"]').setValue('https://example.com')
    await wrapper.find('input[placeholder="Base URL (optional)"]').setValue('')
    await wrapper.find('button.bg-blue-600').trigger('click')
    await new Promise((r) => setTimeout(r, 0))
    expect(wrapper.emitted('update:modelValue')).toEqual([['new-id']])
    vi.unstubAllGlobals()
  })

  it('opts every create-form input out of browser password-manager autofill and gives each field an accessible name', async () => {
    stubFetch([
      { type_name: 'core.httpRequest', display_name: 'HTTP Request', icon: '🌐', category: 'action', description: '', credential_types: [], output_ports: ['main'] },
    ])
    const wrapper = mount(CredentialPicker, { props: { modelValue: null, nodeType: 'core.httpRequest' } })
    await wrapper.find('button').trigger('click')
    await new Promise((r) => setTimeout(r, 0))
    await wrapper.findAll('select')[1].setValue('apiKeyHeader')
    expect(wrapper.find('input[placeholder="Name"]').attributes('autocomplete')).toBe('off')
    const headerName = wrapper.find('input[placeholder="Header Name"]')
    const value = wrapper.find('input[placeholder="Value"]')
    expect(headerName.attributes('autocomplete')).toBe('off')
    expect(value.attributes('autocomplete')).toBe('new-password')
    expect(headerName.attributes('aria-label')).toBe('Header Name')
    expect(value.attributes('aria-label')).toBe('Value')
    vi.unstubAllGlobals()
  })

  it('trims submitted field values and omits a whitespace-only optional field', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn((url: string, options?: RequestInit) => {
        if (url === '/rest/node-types') return Promise.resolve({ ok: true, status: 200, json: async () => [{ type_name: 'ai.agent', display_name: 'AI Agent', icon: '🤖', category: 'action', description: '', credential_types: ['openaiApi'], output_ports: ['main'] }] })
        if (url === '/rest/credentials' && (!options || options.method === undefined)) return Promise.resolve({ ok: true, status: 200, json: async () => [] })
        if (url === '/rest/credential-types') return Promise.resolve({ ok: true, status: 200, json: async () => AI_AGENT_CREDENTIAL_TYPES })
        if (url === '/rest/credentials' && options?.method === 'POST') {
          const body = JSON.parse(options.body as string)
          expect(body.data).toEqual({ api_key: 'openai-key' })
          return Promise.resolve({ ok: true, status: 201, json: async () => ({ id: 'new-id', name: body.name, credential_type: body.credential_type, owner_id: 'u', created_at: '', updated_at: '' }) })
        }
        return Promise.reject(new Error(`unexpected fetch: ${url}`))
      }),
    )
    const wrapper = mount(CredentialPicker, { props: { modelValue: null, nodeType: 'ai.agent' } })
    await wrapper.find('button').trigger('click')
    await new Promise((r) => setTimeout(r, 0))
    await wrapper.find('input[placeholder="Name"]').setValue('My OpenAI Cred')
    await wrapper.find('input[placeholder="API Key"]').setValue('  openai-key \n')
    await wrapper.find('input[placeholder="Base URL (optional)"]').setValue('   ')
    await wrapper.find('button.bg-blue-600').trigger('click')
    await new Promise((r) => setTimeout(r, 0))
    expect(wrapper.emitted('update:modelValue')).toEqual([['new-id']])
    vi.unstubAllGlobals()
  })

  it('clears a validation error when the selected type changes', async () => {
    stubFetch([
      { type_name: 'ai.agent', display_name: 'AI Agent', icon: '🤖', category: 'action', description: '', credential_types: ['openaiApi', 'anthropicApi'], output_ports: ['main'] },
    ], [], AI_AGENT_CREDENTIAL_TYPES)
    const wrapper = mount(CredentialPicker, { props: { modelValue: null, nodeType: 'ai.agent' } })
    await wrapper.find('button').trigger('click')
    await new Promise((r) => setTimeout(r, 0))
    const typeSelect = wrapper.findAll('select')[1]
    await typeSelect.setValue('openaiApi')
    await wrapper.find('button.bg-blue-600').trigger('click')
    expect(wrapper.find('p.text-red-600').text()).toBe('API Key is required.')
    await typeSelect.setValue('anthropicApi')
    expect(wrapper.find('p.text-red-600').exists()).toBe(false)
    vi.unstubAllGlobals()
  })

  it('drops a type picked before node types loaded when it is not among the accepted types', async () => {
    let resolveNodeTypes: (v: unknown) => void = () => {}
    vi.stubGlobal(
      'fetch',
      vi.fn((url: string) => {
        if (url === '/rest/node-types') return new Promise((r) => { resolveNodeTypes = r })
        if (url === '/rest/credentials') return Promise.resolve({ ok: true, status: 200, json: async () => [] })
        if (url === '/rest/credential-types') return Promise.resolve({ ok: true, status: 200, json: async () => [...CREDENTIAL_TYPE_SCHEMAS, ...AI_AGENT_CREDENTIAL_TYPES] })
        return Promise.reject(new Error(`unexpected fetch: ${url}`))
      }),
    )
    const wrapper = mount(CredentialPicker, { props: { modelValue: null, nodeType: 'ai.agent' } })
    await wrapper.find('button').trigger('click')
    await new Promise((r) => setTimeout(r, 0))
    // Node types still loading, so the generic list shows: pick Bearer Token.
    await wrapper.findAll('select')[1].setValue('bearerToken')
    expect(wrapper.find('input[placeholder="Token"]').exists()).toBe(true)
    resolveNodeTypes({ ok: true, status: 200, json: async () => [{ type_name: 'ai.agent', display_name: 'AI Agent', icon: '🤖', category: 'action', description: '', credential_types: ['openaiApi', 'anthropicApi'], output_ports: ['main'] }] })
    await new Promise((r) => setTimeout(r, 0))
    expect(wrapper.find('input[placeholder="Token"]').exists()).toBe(false)
    expect((wrapper.findAll('select')[1].element as HTMLSelectElement).value).toBe('')
    vi.unstubAllGlobals()
  })
})
