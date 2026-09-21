import { describe, it, expect, beforeEach, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { useNodeTypesStore } from './nodeTypes'

describe('useNodeTypesStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('de-duplicates concurrent portsFor calls for the same uncached key', async () => {
    let callCount = 0
    vi.stubGlobal(
      'fetch',
      vi.fn(() => {
        callCount++
        return new Promise((resolve) =>
          setTimeout(() => resolve({ ok: true, status: 200, json: async () => ({ output_ports: ['main'] }) }), 0),
        )
      }),
    )
    const store = useNodeTypesStore()
    const [a, b] = await Promise.all([
      store.portsFor('core.set', {}),
      store.portsFor('core.set', {}),
    ])
    expect(callCount).toBe(1)
    expect(a).toEqual(['main'])
    expect(b).toEqual(['main'])
    vi.unstubAllGlobals()
  })

  it('URL-encodes the type name in the output-ports request path', async () => {
    let requestedUrl = ''
    vi.stubGlobal(
      'fetch',
      vi.fn((url: string) => {
        requestedUrl = url
        return Promise.resolve({ ok: true, status: 200, json: async () => ({ output_ports: ['main'] }) })
      }),
    )
    const store = useNodeTypesStore()
    await store.portsFor('a/b c', {})
    expect(requestedUrl).toBe(`/rest/node-types/${encodeURIComponent('a/b c')}/output-ports`)
    vi.unstubAllGlobals()
  })
})
