import { describe, it, expect, beforeEach, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import CredentialForm from './CredentialForm.vue'

const SCHEMAS = [
  { credential_type: 'apiKeyHeader', display_name: 'API Key (Header)', generic: true, fields: [
    { name: 'header_name', label: 'Header Name', field_type: 'text', required: true },
    { name: 'value', label: 'Value', field_type: 'password', required: true },
  ] },
]

function stub(detail: unknown, onPatch: (body: unknown) => void) {
  vi.stubGlobal('fetch', vi.fn((url: string, options?: RequestInit) => {
    if (url === '/rest/credential-types') return Promise.resolve({ ok: true, status: 200, json: async () => SCHEMAS })
    if (url === '/rest/credentials/c1' && !options?.method) return Promise.resolve({ ok: true, status: 200, json: async () => detail })
    if (url === '/rest/credentials/c1' && options?.method === 'PATCH') {
      onPatch(JSON.parse(options.body as string))
      return Promise.resolve({ ok: true, status: 200, json: async () => ({ ...(detail as object), used_by: 0 }) })
    }
    if (url === '/rest/credentials') return Promise.resolve({ ok: true, status: 200, json: async () => [] })
    return Promise.reject(new Error(`unexpected fetch: ${url}`))
  }))
}

describe('CredentialForm', () => {
  beforeEach(() => setActivePinia(createPinia()))

  it('edit mode pre-fills text fields and leaves secrets blank with the unchanged placeholder', async () => {
    stub({ id: 'c1', name: 'My header', credential_type: 'apiKeyHeader', owner_id: 'u', created_at: '', updated_at: '', used_by: 2, fields: { header_name: 'X-Key' } }, () => {})
    const wrapper = mount(CredentialForm, { props: { mode: 'edit', credentialId: 'c1' } })
    await flushPromises()
    expect((wrapper.find('input[aria-label="Name"]').element as HTMLInputElement).value).toBe('My header')
    expect((wrapper.find('input[aria-label="Header Name"]').element as HTMLInputElement).value).toBe('X-Key')
    const secret = wrapper.find('input[aria-label="Value"]')
    expect((secret.element as HTMLInputElement).value).toBe('')
    expect(secret.attributes('placeholder')).toBe('•••••• (unchanged)')
    expect(wrapper.text()).toContain('apiKeyHeader')
    vi.unstubAllGlobals()
  })

  it('edit mode submits the name and only non-empty field values', async () => {
    let patched: unknown = null
    stub({ id: 'c1', name: 'My header', credential_type: 'apiKeyHeader', owner_id: 'u', created_at: '', updated_at: '', used_by: 0, fields: { header_name: 'X-Key' } }, (b) => { patched = b })
    const wrapper = mount(CredentialForm, { props: { mode: 'edit', credentialId: 'c1' } })
    await flushPromises()
    await wrapper.find('input[aria-label="Name"]').setValue('Renamed')
    await wrapper.find('button.bg-blue-600').trigger('click')
    await flushPromises()
    expect(patched).toEqual({ name: 'Renamed', data: { header_name: 'X-Key' } })
    expect(wrapper.emitted('saved')).toBeTruthy()
    vi.unstubAllGlobals()
  })
})