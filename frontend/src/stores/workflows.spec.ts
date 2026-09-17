import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { useWorkflowsStore } from './workflows'

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

describe('workflows store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('fetchAll populates workflows from the API', async () => {
    mockFetchOnce([{ id: '1', name: 'a' }])
    const store = useWorkflowsStore()
    await store.fetchAll()
    expect(store.workflows).toHaveLength(1)
    expect(store.loading).toBe(false)
    vi.unstubAllGlobals()
  })

  it('create appends the new workflow', async () => {
    mockFetchOnce({ id: '2', name: 'new one' })
    const store = useWorkflowsStore()
    const created = await store.create('new one')
    expect(created.id).toBe('2')
    expect(store.workflows).toContainEqual(created)
    vi.unstubAllGlobals()
  })

  it('remove drops the workflow from local state', async () => {
    const store = useWorkflowsStore()
    store.workflows = [{ id: '1', name: 'a' } as never]
    mockFetchOnce(undefined, 204)
    await store.remove('1')
    expect(store.workflows).toHaveLength(0)
    vi.unstubAllGlobals()
  })
})
