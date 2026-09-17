import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { useAuthStore } from './auth'

describe('auth store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    localStorage.clear()
  })

  it('starts unauthenticated when no token is stored', () => {
    const store = useAuthStore()
    expect(store.isAuthenticated).toBe(false)
  })

  it('login stores the token and marks authenticated', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        status: 200,
        json: async () => ({ token: 'abc123' }),
      }),
    )
    const store = useAuthStore()
    await store.login('a@b.com', 'password')
    expect(store.isAuthenticated).toBe(true)
    expect(localStorage.getItem('r8r_token')).toBe('abc123')
    vi.unstubAllGlobals()
  })

  it('logout clears the token', () => {
    localStorage.setItem('r8r_token', 'abc123')
    const store = useAuthStore()
    store.logout()
    expect(store.isAuthenticated).toBe(false)
    expect(localStorage.getItem('r8r_token')).toBeNull()
  })
})
