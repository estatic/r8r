import { describe, it, expect, vi, beforeEach } from 'vitest'
import { useLiveExecutionSocket } from './useLiveExecutionSocket'

class MockWebSocket {
  static instances: MockWebSocket[] = []
  listeners: Record<string, Array<(e: unknown) => void>> = {}
  sent: string[] = []
  constructor(public url: string) {
    MockWebSocket.instances.push(this)
  }
  addEventListener(type: string, cb: (e: unknown) => void) {
    ;(this.listeners[type] ??= []).push(cb)
  }
  send(data: string) {
    this.sent.push(data)
  }
  close() {
    this.listeners['close']?.forEach((cb) => cb({}))
  }
  drop() {
    this.listeners['close']?.forEach((cb) => cb({}))
  }
  emitOpen() {
    this.listeners['open']?.forEach((cb) => cb({}))
  }
  emitMessage(data: unknown) {
    this.listeners['message']?.forEach((cb) => cb({ data: JSON.stringify(data) }))
  }
}

describe('useLiveExecutionSocket', () => {
  beforeEach(() => {
    MockWebSocket.instances = []
    vi.stubGlobal('WebSocket', MockWebSocket)
    localStorage.setItem('r8r_token', 'test-token')
  })

  it('sends the auth frame once the socket opens', () => {
    const { connect } = useLiveExecutionSocket('wf-1')
    connect()
    const ws = MockWebSocket.instances[0]
    ws.emitOpen()
    expect(ws.sent).toEqual([JSON.stringify({ token: 'test-token' })])
  })

  it('adopts a new execution_id and fills node_outputs from node_finished and node_skipped events', () => {
    const { execution, connect } = useLiveExecutionSocket('wf-1')
    connect()
    const ws = MockWebSocket.instances[0]

    ws.emitMessage({ type: 'node_started', execution_id: 'e1', workflow_id: 'wf-1', node_id: 'trigger' })
    expect(execution.value?.id).toBe('e1')
    expect(execution.value?.status).toBe('Running')

    ws.emitMessage({
      type: 'node_finished',
      execution_id: 'e1',
      workflow_id: 'wf-1',
      node_id: 'trigger',
      items: [{ json: {}, binary: {} }],
    })
    expect(execution.value?.node_outputs['trigger']).toEqual([{ json: {}, binary: {} }])

    ws.emitMessage({
      type: 'node_skipped',
      execution_id: 'e1',
      workflow_id: 'wf-1',
      node_id: 'disabled_node',
      items: [{ json: { passthrough: true }, binary: {} }],
    })
    expect(execution.value?.node_outputs['disabled_node']).toEqual([{ json: { passthrough: true }, binary: {} }])

    ws.emitMessage({ type: 'execution_finished', execution_id: 'e1', workflow_id: 'wf-1', status: 'Success' })
    expect(execution.value?.status).toBe('Success')
    expect(execution.value?.finished_at).not.toBeNull()
  })

  it('marks a reused node and keeps an error\'s full text on its run', () => {
    const { execution, connect } = useLiveExecutionSocket('wf-1')
    connect()
    const ws = MockWebSocket.instances[0]
    ws.emitMessage({ execution_id: 'e1', workflow_id: 'wf-1', type: 'node_finished', node_id: 't', items: [], counts: { '0': 0 }, reused: true })
    ws.emitMessage({ execution_id: 'e1', workflow_id: 'wf-1', type: 'node_errored', node_id: 'c', error: 'boom\n  at main (line 1:7)' })
    expect(execution.value?.node_runs?.t.reused).toBe(true)
    expect(execution.value?.node_runs?.c.error).toBe('boom\n  at main (line 1:7)')
  })

  it('records a node_errored event as a synthesized error item', () => {
    const { execution, connect } = useLiveExecutionSocket('wf-1')
    connect()
    const ws = MockWebSocket.instances[0]

    ws.emitMessage({ type: 'node_started', execution_id: 'e1', workflow_id: 'wf-1', node_id: 'set1' })
    ws.emitMessage({ type: 'node_errored', execution_id: 'e1', workflow_id: 'wf-1', node_id: 'set1', error: 'boom' })

    expect(execution.value?.node_outputs['set1']).toEqual([{ json: { error: 'boom' }, binary: {} }])
  })

  it('a new execution_id supersedes a stale one, and a late event for the superseded id is ignored', () => {
    const { execution, connect } = useLiveExecutionSocket('wf-1')
    connect()
    const ws = MockWebSocket.instances[0]

    ws.emitMessage({ type: 'node_finished', execution_id: 'e1', workflow_id: 'wf-1', node_id: 'a', items: [] })
    ws.emitMessage({ type: 'node_started', execution_id: 'e2', workflow_id: 'wf-1', node_id: 'b' })

    expect(execution.value?.id).toBe('e2')
    expect(execution.value?.node_outputs['a']).toBeUndefined()

    // A stray, out-of-order event for the now-superseded e1 must not revert the view back to e1.
    ws.emitMessage({
      type: 'node_finished',
      execution_id: 'e1',
      workflow_id: 'wf-1',
      node_id: 'a',
      items: [{ json: {}, binary: {} }],
    })
    expect(execution.value?.id).toBe('e2')
  })

  it('a live event never mutates an externally-assigned execution object (e.g. a selected historical run)', () => {
    const { execution, connect } = useLiveExecutionSocket('wf-1')
    connect()
    const ws = MockWebSocket.instances[0]

    ws.emitMessage({ type: 'node_started', execution_id: 'e1', workflow_id: 'wf-1', node_id: 'trigger' })
    expect(execution.value?.id).toBe('e1')

    // Simulate the user selecting a different, unrelated historical execution
    // in the UI (WorkflowEditorView's @select handler does exactly this: it
    // reassigns the ref directly, bypassing applyEvent).
    const historical = {
      id: 'h1',
      workflow_id: 'wf-1',
      status: 'Success' as const,
      mode: 'Manual' as const,
      node_outputs: { old: [{ json: { keep: true }, binary: {} }] },
      started_at: 'earlier',
      finished_at: 'earlier',
    }
    execution.value = historical

    // A live event for the original in-flight execution (e1) must not write
    // into the historical object -- it must swap to a fresh e1 object instead.
    ws.emitMessage({
      type: 'node_finished',
      execution_id: 'e1',
      workflow_id: 'wf-1',
      node_id: 'trigger',
      items: [{ json: {}, binary: {} }],
    })

    expect(execution.value?.id).toBe('e1')
    expect(historical.node_outputs).toEqual({ old: [{ json: { keep: true }, binary: {} }] })
  })

  it('joining a run midway fills in the nodes that ran before from the server', async () => {
    const earlier = { status: 'success', counts: { '0': 1 }, outputs: [[{ json: { text: 'hi' }, binary: {} }]] }
    vi.stubGlobal('fetch', vi.fn(async () => ({
      ok: true,
      status: 200,
      json: async () => ({
        id: 'e1', workflow_id: 'wf-1', status: 'Running', mode: 'Trigger', started_at: 's', finished_at: null,
        node_outputs: { trigger: [{ json: { text: 'hi' }, binary: {} }], agent: [] },
        node_runs: { trigger: earlier, agent: { status: 'success', counts: {} } },
      }),
    })))
    const { execution, connect } = useLiveExecutionSocket('wf-1')
    connect()
    const ws = MockWebSocket.instances[0]
    ws.emitMessage({ type: 'node_started', execution_id: 'e1', workflow_id: 'wf-1', node_id: 'agent' })
    await vi.waitFor(() => expect(execution.value?.node_runs?.trigger).toEqual(earlier))
    expect(execution.value?.node_outputs.trigger).toEqual([{ json: { text: 'hi' }, binary: {} }])
    // What the socket saw is newer than the server's copy: kept.
    expect(execution.value?.node_runs?.agent.status).toBe('running')
    expect(execution.value?.mode).toBe('Trigger')
    vi.unstubAllGlobals()
  })

  it('shows the server\'s final copy once a run finishes', async () => {
    const final = { id: 'e1', workflow_id: 'wf-1', status: 'Success', mode: 'Trigger', started_at: 's', finished_at: 'f', node_outputs: { a: [], b: [] } }
    vi.stubGlobal('fetch', vi.fn(async () => ({ ok: true, status: 200, json: async () => final })))
    const { execution, connect } = useLiveExecutionSocket('wf-1')
    connect()
    MockWebSocket.instances[0].emitMessage({ type: 'execution_finished', execution_id: 'e1', workflow_id: 'wf-1', status: 'Success' })
    await vi.waitFor(() => expect(execution.value).toEqual(final))
    vi.unstubAllGlobals()
  })

  it('reconnects a dropped socket, but not one it closed itself', () => {
    vi.useFakeTimers()
    const { connect, disconnect } = useLiveExecutionSocket('wf-1')
    connect()
    MockWebSocket.instances[0].drop()
    vi.advanceTimersByTime(2000)
    expect(MockWebSocket.instances).toHaveLength(2)
    disconnect()
    vi.advanceTimersByTime(5000)
    expect(MockWebSocket.instances).toHaveLength(2)
    vi.useRealTimers()
  })

  it('does not connect when there is no stored token', () => {
    localStorage.removeItem('r8r_token')
    const { connect } = useLiveExecutionSocket('wf-1')
    connect()
    expect(MockWebSocket.instances).toHaveLength(0)
  })
})
