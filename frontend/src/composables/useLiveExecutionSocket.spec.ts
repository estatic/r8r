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
  close() {}
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

  it('adopts a new execution_id and fills node_outputs from node_finished/skipped events', () => {
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

    ws.emitMessage({ type: 'execution_finished', execution_id: 'e1', workflow_id: 'wf-1', status: 'Success' })
    expect(execution.value?.status).toBe('Success')
    expect(execution.value?.finished_at).not.toBeNull()
  })

  it('a new execution_id supersedes a stale one', () => {
    const { execution, connect } = useLiveExecutionSocket('wf-1')
    connect()
    const ws = MockWebSocket.instances[0]

    ws.emitMessage({ type: 'node_finished', execution_id: 'e1', workflow_id: 'wf-1', node_id: 'a', items: [] })
    ws.emitMessage({ type: 'node_started', execution_id: 'e2', workflow_id: 'wf-1', node_id: 'b' })

    expect(execution.value?.id).toBe('e2')
    expect(execution.value?.node_outputs['a']).toBeUndefined()
  })

  it('does not connect when there is no stored token', () => {
    localStorage.removeItem('r8r_token')
    const { connect } = useLiveExecutionSocket('wf-1')
    connect()
    expect(MockWebSocket.instances).toHaveLength(0)
  })
})
