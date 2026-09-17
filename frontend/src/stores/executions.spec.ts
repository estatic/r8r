import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { useExecutionsStore } from './executions'

function mockFetchOnce(body: unknown, status = 200) {
  vi.stubGlobal(
    'fetch',
    vi.fn().mockResolvedValue({
      ok: status < 400,
      status,
      json: async () => body,
    }),
  )
}

describe('executions store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('fetchHistory populates history from the API', async () => {
    mockFetchOnce([{ id: 'e2' }, { id: 'e1' }])
    const store = useExecutionsStore()
    await store.fetchHistory('wf-1')
    expect(store.history).toHaveLength(2)
    expect(store.history[0].id).toBe('e2')
    vi.unstubAllGlobals()
  })

  it('fetchHistory requests the given workflow and limit', async () => {
    mockFetchOnce([])
    const fetchSpy = vi.mocked(globalThis.fetch)
    const store = useExecutionsStore()
    await store.fetchHistory('wf-1', 10)
    expect(fetchSpy.mock.calls[0][0]).toBe('/rest/workflows/wf-1/executions?limit=10')
    vi.unstubAllGlobals()
  })
})
