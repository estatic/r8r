import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { useCredentialsStore } from './credentials'

describe('credentials store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('create appends the new credential summary (never the secret data)', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        status: 201,
        json: async () => ({ id: '1', name: 'my-bot', credential_type: 'telegramApi', owner_id: 'u1', created_at: 'x', updated_at: 'x' }),
      }),
    )
    const store = useCredentialsStore()
    const summary = await store.create('my-bot', 'telegramApi', { bot_token: 'secret' })
    expect(summary.id).toBe('1')
    expect('data' in summary).toBe(false)
    expect(store.credentials).toContainEqual(summary)
    vi.unstubAllGlobals()
  })
})
