import { describe, it, expect, beforeEach, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { useCredentialTypesStore } from './credentialTypes'

const SCHEMAS = [
  { credential_type: 'telegramApi', display_name: 'Telegram Bot', generic: false, fields: [{ name: 'bot_token', label: 'Bot Token', field_type: 'password', required: true }] },
  { credential_type: 'bearerToken', display_name: 'Bearer Token', generic: true, fields: [{ name: 'token', label: 'Token', field_type: 'password', required: true }] },
]

describe('useCredentialTypesStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('fetches all known credential type schemas once', async () => {
    let callCount = 0
    vi.stubGlobal(
      'fetch',
      vi.fn(() => {
        callCount++
        return Promise.resolve({ ok: true, status: 200, json: async () => SCHEMAS })
      }),
    )
    const store = useCredentialTypesStore()
    await store.fetchAll()
    await store.fetchAll()
    expect(callCount).toBe(1)
    expect(store.types).toEqual(SCHEMAS)
    expect(store.loaded).toBe(true)
    vi.unstubAllGlobals()
  })
})
