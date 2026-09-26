import { describe, it, expect, beforeEach, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import CredentialsView from './CredentialsView.vue'

const CREDS = [
  { id: 'c1', name: 'Bot', credential_type: 'telegramApi', owner_id: 'u', created_at: '', updated_at: '2026-09-23T10:00:00Z', used_by: 2 },
  { id: 'c2', name: 'Spare', credential_type: 'bearerToken', owner_id: 'u', created_at: '', updated_at: '2026-09-23T10:00:00Z', used_by: 0 },
]

function stub(deleteStatus: number, deleteBody = '') {
  let list = [...CREDS]
  vi.stubGlobal('fetch', vi.fn((url: string, options?: RequestInit) => {
    if (url === '/rest/r8r/credentials') return Promise.resolve({ ok: true, status: 200, json: async () => list })
    if (url === '/rest/r8r/credential-types') return Promise.resolve({ ok: true, status: 200, json: async () => [] })
    if (options?.method === 'DELETE') {
      if (deleteStatus === 204) {
        list = list.filter((c) => !url.endsWith(c.id))
        return Promise.resolve({ ok: true, status: 204, json: async () => undefined })
      }
      return Promise.resolve({ ok: false, status: deleteStatus, text: async () => deleteBody })
    }
    return Promise.reject(new Error(`unexpected fetch: ${url}`))
  }))
}

const mountView = () => mount(CredentialsView, { global: { stubs: { RouterLink: true } } })

describe('CredentialsView', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.spyOn(window, 'confirm').mockReturnValue(true)
  })

  it('lists credentials with their usage counts', async () => {
    stub(204)
    const wrapper = mountView()
    await flushPromises()
    const rows = wrapper.findAll('[data-testid="credential-row"]')
    expect(rows).toHaveLength(2)
    expect(rows[0].text()).toContain('Bot')
    expect(rows[0].text()).toContain('used by 2 workflows')
    expect(rows[1].text()).toContain('not used')
    vi.unstubAllGlobals()
  })

  it('names the workflows when deletion is refused', async () => {
    stub(409, JSON.stringify({ error: 'credential is in use', workflows: [{ id: 'w1', name: 'Daily digest' }] }))
    const wrapper = mountView()
    await flushPromises()
    await wrapper.findAll('[data-testid="delete-credential"]')[0].trigger('click')
    await flushPromises()
    expect(wrapper.text()).toContain('Can\'t delete "Bot": used by Daily digest. Remove it from those workflows first.')
    vi.unstubAllGlobals()
  })

  it('falls back to a generic message for an unexpected 409 body', async () => {
    stub(409, 'not json')
    const wrapper = mountView()
    await flushPromises()
    await wrapper.findAll('[data-testid="delete-credential"]')[0].trigger('click')
    await flushPromises()
    expect(wrapper.text()).toContain('Failed to delete credential.')
    vi.unstubAllGlobals()
  })

  it('removes the row after a successful delete', async () => {
    stub(204)
    const wrapper = mountView()
    await flushPromises()
    await wrapper.findAll('[data-testid="delete-credential"]')[1].trigger('click')
    await flushPromises()
    expect(wrapper.findAll('[data-testid="credential-row"]')).toHaveLength(1)
    vi.unstubAllGlobals()
  })
})