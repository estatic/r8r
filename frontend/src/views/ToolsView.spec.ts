import { describe, it, expect, beforeEach, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import ToolsView from './ToolsView.vue'

const TOOLS = [
  { id: 't1', name: 'search', description: 'web search', node_type: 'core.httpRequest', argument_schema: { type: 'object' }, parameters: {}, created_at: '', updated_at: '', used_by: 2 },
  { id: 't2', name: 'double', description: 'doubles n', node_type: 'core.code', argument_schema: { type: 'object' }, parameters: {}, created_at: '', updated_at: '', used_by: 0 },
]

function stub(opts: { deleteStatus?: number; deleteBody?: string; onPost?: (body: unknown) => void } = {}) {
  let list = [...TOOLS]
  vi.stubGlobal('fetch', vi.fn((url: string, options?: RequestInit) => {
    if (url === '/rest/tools' && !options?.method) return Promise.resolve({ ok: true, status: 200, json: async () => list })
    if (url === '/rest/tools' && options?.method === 'POST') {
      const body = JSON.parse(options.body as string)
      opts.onPost?.(body)
      return Promise.resolve({ ok: true, status: 201, json: async () => ({ ...body, id: 't9', created_at: '', updated_at: '' }) })
    }
    if (options?.method === 'DELETE') {
      if ((opts.deleteStatus ?? 204) === 204) {
        list = list.filter((t) => !url.endsWith(t.id))
        return Promise.resolve({ ok: true, status: 204, json: async () => undefined })
      }
      return Promise.resolve({ ok: false, status: opts.deleteStatus, text: async () => opts.deleteBody ?? '' })
    }
    if (url === '/rest/credentials' || url === '/rest/node-types' || url === '/rest/credential-types') {
      return Promise.resolve({ ok: true, status: 200, json: async () => [] })
    }
    return Promise.reject(new Error(`unexpected fetch: ${url}`))
  }))
}

const mountView = () => mount(ToolsView, { global: { stubs: { RouterLink: true } } })

describe('ToolsView', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.spyOn(window, 'confirm').mockReturnValue(true)
  })

  it('lists tools with their usage counts', async () => {
    stub()
    const wrapper = mountView()
    await flushPromises()
    const rows = wrapper.findAll('[data-testid="tool-row"]')
    expect(rows).toHaveLength(2)
    expect(rows[0].text()).toContain('search')
    expect(rows[0].text()).toContain('HTTP Request')
    expect(rows[0].text()).toContain('used by 2 workflows')
    expect(rows[1].text()).toContain('not used')
    vi.unstubAllGlobals()
  })

  it('names the workflows when deletion is refused', async () => {
    stub({ deleteStatus: 409, deleteBody: JSON.stringify({ error: 'tool is in use', workflows: [{ id: 'w1', name: 'Support bot' }] }) })
    const wrapper = mountView()
    await flushPromises()
    await wrapper.findAll('[data-testid="delete-tool"]')[0].trigger('click')
    await flushPromises()
    expect(wrapper.text()).toContain('Can\'t delete "search": used by Support bot. Remove it from those agents first.')
    vi.unstubAllGlobals()
  })

  it('removes the row after a successful delete', async () => {
    stub()
    const wrapper = mountView()
    await flushPromises()
    await wrapper.findAll('[data-testid="delete-tool"]')[1].trigger('click')
    await flushPromises()
    expect(wrapper.findAll('[data-testid="tool-row"]')).toHaveLength(1)
    vi.unstubAllGlobals()
  })

  it('pre-fills the HTTP template for a new tool and posts the built schema', async () => {
    let posted: Record<string, unknown> | null = null
    stub({ onPost: (b) => { posted = b as Record<string, unknown> } })
    const wrapper = mountView()
    await flushPromises()
    await wrapper.find('[data-testid="new-tool"]').trigger('click')
    await flushPromises()
    const params = wrapper.find('textarea[aria-label="Parameters (JSON)"]')
    expect((params.element as HTMLTextAreaElement).value).toContain('{{ encodeURIComponent($args.query) }}')

    await wrapper.find('input[aria-label="Name"]').setValue('web_search')
    await wrapper.find('textarea[aria-label="Description"]').setValue('Search the web')
    await wrapper.find('[data-testid="add-argument"]').trigger('click')
    await wrapper.find('input[aria-label="Argument name"]').setValue('query')
    await wrapper.find('input[aria-label="Argument required"]').setValue(true)
    await wrapper.find('[data-testid="save-tool"]').trigger('click')
    await flushPromises()

    expect(posted).toMatchObject({
      name: 'web_search',
      description: 'Search the web',
      node_type: 'core.httpRequest',
      argument_schema: { type: 'object', properties: { query: { type: 'string' } }, required: ['query'] },
      parameters: { method: 'GET', url: 'https://api.example.com/search?q={{ encodeURIComponent($args.query) }}' },
    })
    vi.unstubAllGlobals()
  })

  it('rejects an invalid tool name before saving', async () => {
    let posted = false
    stub({ onPost: () => { posted = true } })
    const wrapper = mountView()
    await flushPromises()
    await wrapper.find('[data-testid="new-tool"]').trigger('click')
    await wrapper.find('input[aria-label="Name"]').setValue('has space')
    await wrapper.find('[data-testid="save-tool"]').trigger('click')
    await flushPromises()
    expect(wrapper.text()).toContain('Name must be 1-64 letters, digits, _ or -.')
    expect(posted).toBe(false)
    vi.unstubAllGlobals()
  })
})
