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
    if (url.startsWith('/rest/executions/')) {
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

  it('replaces the socket copy with the server copy once the socket finishes the run', async () => {
    const serverCopy = exec('e1', 'Success', { set1: [{ json: { full: true }, binary: {} }], set2: [] })
    stubFetch(exec('e1', 'Running'), [serverCopy])
    const execution = ref<Execution | null>(null)
    const run = useWorkflowRun('wf', execution)
    await run.execute()
    // Socket finishes the run but missed some node events (e.g. panel closed mid-run).
    execution.value!.status = 'Success'
    await nextTick()
    await vi.advanceTimersByTimeAsync(0)
    expect(execution.value).toEqual(serverCopy)
  })
})
