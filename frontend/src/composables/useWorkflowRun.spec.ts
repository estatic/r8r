import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { effectScope, nextTick, ref } from 'vue'
import { useWorkflowRun } from './useWorkflowRun'
import type { Execution } from '../types/domain'

function exec(id: string, status: Execution['status'], outputs: Execution['node_outputs'] = {}): Execution {
  return { id, workflow_id: 'wf', status, mode: 'Manual', node_outputs: outputs, started_at: '', finished_at: null }
}

function stubFetch(postResult: Execution, getResults: Execution[] = []) {
  const fetchMock = vi.fn((url: string, options?: RequestInit) => {
    if (options?.method === 'POST') return Promise.resolve({ ok: true, status: 202, json: async () => postResult })
    if (url.startsWith('/rest/r8r/executions/')) {
      const next = getResults.shift() ?? postResult
      return Promise.resolve({ ok: true, status: 200, json: async () => next })
    }
    return Promise.reject(new Error(`unexpected fetch: ${url}`))
  })
  vi.stubGlobal('fetch', fetchMock)
  return fetchMock
}

describe('useWorkflowRun', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })
  afterEach(() => {
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })

  it('stays executing after the 202 until the run finishes over the socket', async () => {
    stubFetch(exec('e1', 'Running'))
    const execution = ref<Execution | null>(null)
    const run = useWorkflowRun('wf', execution)
    await run.execute()
    expect(run.executing.value).toBe(true)
    expect(execution.value?.id).toBe('e1')
    execution.value!.status = 'Success' // what the socket's execution_finished does
    await nextTick()
    expect(run.executing.value).toBe(false)
  })

  it('asks for a fresh run only when told to', async () => {
    const fetchMock = stubFetch(exec('e1', 'Running'))
    const run = useWorkflowRun('wf', ref<Execution | null>(null))
    await run.execute()
    expect(fetchMock.mock.calls[0][0]).toBe('/rest/r8r/workflows/wf/execute')
    run.stop()
    await run.execute(true)
    expect(fetchMock.mock.calls[fetchMock.mock.calls.length - 1][0]).toBe('/rest/r8r/workflows/wf/execute?fresh=true')
  })

  it('keeps socket node outputs when the POST response arrives after socket events', async () => {
    stubFetch(exec('e1', 'Running'))
    const execution = ref<Execution | null>(exec('e1', 'Running', { set1: [{ json: { a: 1 }, binary: {} }] }))
    const run = useWorkflowRun('wf', execution)
    await run.execute()
    expect(execution.value?.node_outputs.set1).toEqual([{ json: { a: 1 }, binary: {} }])
  })

  it('finishes immediately when the socket already reported completion', async () => {
    stubFetch(exec('e1', 'Running'))
    const execution = ref<Execution | null>(exec('e1', 'Success'))
    const run = useWorkflowRun('wf', execution)
    await run.execute()
    expect(run.executing.value).toBe(false)
  })

  it('falls back to polling the execution when no socket event arrives', async () => {
    const fetchMock = stubFetch(exec('e1', 'Running'), [exec('e1', 'Running'), exec('e1', 'Success', { set1: [] })])
    const execution = ref<Execution | null>(null)
    const run = useWorkflowRun('wf', execution, 2000)
    await run.execute()
    await vi.advanceTimersByTimeAsync(2000)
    expect(run.executing.value).toBe(true)
    await vi.advanceTimersByTimeAsync(2000)
    expect(execution.value?.status).toBe('Success')
    expect(run.executing.value).toBe(false)
    const calls = fetchMock.mock.calls.length
    await vi.advanceTimersByTimeAsync(6000)
    expect(fetchMock.mock.calls.length).toBe(calls) // polling stopped
  })

  it('stop() clears polling', async () => {
    const fetchMock = stubFetch(exec('e1', 'Running'))
    const execution = ref<Execution | null>(null)
    const run = useWorkflowRun('wf', execution, 2000)
    await run.execute()
    run.stop()
    const calls = fetchMock.mock.calls.length
    await vi.advanceTimersByTimeAsync(6000)
    expect(fetchMock.mock.calls.length).toBe(calls)
    expect(run.executing.value).toBe(false)
  })

  it('rethrows a failed POST and is not left executing', async () => {
    vi.stubGlobal('fetch', vi.fn(() => Promise.resolve({ ok: false, status: 500, text: async () => 'boom' })))
    const run = useWorkflowRun('wf', ref<Execution | null>(null))
    await expect(run.execute()).rejects.toThrow('boom')
    expect(run.executing.value).toBe(false)
  })

  it('does not start polling when its scope is disposed while the POST is in flight', async () => {
    let resolvePost: (v: unknown) => void = () => {}
    const fetchMock = vi.fn((_url: string, options?: RequestInit) =>
      options?.method === 'POST'
        ? new Promise((r) => { resolvePost = r })
        : Promise.resolve({ ok: true, status: 200, json: async () => exec('e1', 'Running') }),
    )
    vi.stubGlobal('fetch', fetchMock)
    const scope = effectScope()
    const run = scope.run(() => useWorkflowRun('wf', ref<Execution | null>(null), 2000))!
    const pending = run.execute()
    scope.stop() // user leaves the editor
    resolvePost({ ok: true, status: 202, json: async () => exec('e1', 'Running') })
    await pending
    await vi.advanceTimersByTimeAsync(6000)
    expect(fetchMock.mock.calls.length).toBe(1) // only the POST
    expect(run.executing.value).toBe(false)
  })

  it('stops tracking once the socket finishes the run (the socket fetches the final copy)', async () => {
    const fetchMock = stubFetch(exec('e1', 'Running'))
    const execution = ref<Execution | null>(null)
    const run = useWorkflowRun('wf', execution)
    await run.execute()
    execution.value!.status = 'Success'
    await nextTick()
    expect(run.executing.value).toBe(false)
    await vi.advanceTimersByTimeAsync(6000)
    expect(fetchMock.mock.calls.length).toBe(1) // only the POST: no polling left
  })

  it('stops a run on screen that a trigger started', async () => {
    const fetchMock = vi.fn((_url: string, _options?: RequestInit) => Promise.resolve({ ok: true, status: 204 }))
    vi.stubGlobal('fetch', fetchMock)
    const execution = ref<Execution | null>(exec('t1', 'Running'))
    await useWorkflowRun('wf', execution).cancel()
    expect(fetchMock.mock.calls.some(([url, o]) => url === '/rest/r8r/executions/t1/stop' && o?.method === 'POST')).toBe(true)
  })

  it('stopping while the run is still being started (e.g. waiting for a Telegram message) abandons the request quietly', async () => {
    let signal: AbortSignal | undefined
    const fetchMock = vi.fn((_url: string, options?: RequestInit) => {
      signal = options?.signal ?? undefined
      return new Promise((_resolve, reject) => {
        signal?.addEventListener('abort', () => reject(new DOMException('aborted', 'AbortError')))
      })
    })
    vi.stubGlobal('fetch', fetchMock)
    const run = useWorkflowRun('wf', ref<Execution | null>(null))
    const pending = run.execute()
    expect(run.executing.value).toBe(true)
    await run.cancel()
    await expect(pending).resolves.toBeUndefined()
    expect(signal?.aborted).toBe(true)
    expect(run.executing.value).toBe(false)
  })

  it('stopping a started run asks the server to stop it; the run then ends as Canceled', async () => {
    const fetchMock = vi.fn((url: string, options?: RequestInit) => {
      if (url === '/rest/r8r/executions/e1/stop') return Promise.resolve({ ok: true, status: 204 })
      if (options?.method === 'POST') return Promise.resolve({ ok: true, status: 202, json: async () => exec('e1', 'Running') })
      return Promise.resolve({ ok: true, status: 200, json: async () => exec('e1', 'Canceled') })
    })
    vi.stubGlobal('fetch', fetchMock)
    const execution = ref<Execution | null>(null)
    const run = useWorkflowRun('wf', execution)
    await run.execute()
    await run.cancel()
    expect(fetchMock.mock.calls.some(([url, o]) => url === '/rest/r8r/executions/e1/stop' && o?.method === 'POST')).toBe(true)
    await vi.advanceTimersByTimeAsync(2000)
    expect(execution.value?.status).toBe('Canceled')
    expect(run.executing.value).toBe(false)
  })
})
