import { ref, type Ref } from 'vue'
import { api, getToken } from '../api/client'
import type { Execution, ExecutionStatus, Item } from '../types/domain'

export type LiveExecutionEvent = { execution_id: string; workflow_id: string } & (
  | { type: 'node_started'; node_id: string }
  | { type: 'node_finished'; node_id: string; items: Item[]; counts?: Record<string, number>; reused?: boolean }
  | { type: 'node_errored'; node_id: string; error: string; counts?: Record<string, number> }
  | { type: 'node_skipped'; node_id: string; items: Item[]; counts?: Record<string, number> }
  | { type: 'execution_finished'; status: ExecutionStatus }
)

export interface LiveExecutionSocket {
  execution: Ref<Execution | null>
  connect: () => void
  disconnect: () => void
}

/** How long to wait before reconnecting a dropped socket (e.g. the server restarted). */
const RECONNECT_MS = 2000

export function useLiveExecutionSocket(workflowId: string): LiveExecutionSocket {
  const execution = ref<Execution | null>(null)
  let socket: WebSocket | null = null
  let wanted = false
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null
  const supersededExecutionIds = new Set<string>()

  /**
   * Fills in what the socket didn't see from the server's copy: nodes that
   * ran before it joined the run (a trigger started it, the editor was
   * opened mid-run) or events it dropped. A finished run is shown as the
   * server has it; a running one only gets the nodes it's missing.
   */
  async function catchUp(id: string) {
    let server: Execution
    try {
      server = await api.get<Execution>(`/rest/r8r/executions/${id}`)
    } catch {
      return // keep what the socket has
    }
    const current = execution.value
    if (!current || current.id !== id) return
    if (server.status !== 'Running') {
      execution.value = server
      return
    }
    current.mode = server.mode
    current.started_at = server.started_at
    const runs = (current.node_runs ??= {})
    for (const [nodeId, run] of Object.entries(server.node_runs ?? {})) {
      if (!(nodeId in runs)) runs[nodeId] = run
    }
    for (const [nodeId, items] of Object.entries(server.node_outputs)) {
      if (!(nodeId in current.node_outputs)) current.node_outputs[nodeId] = items
    }
  }

  function applyEvent(event: LiveExecutionEvent) {
    if (supersededExecutionIds.has(event.execution_id)) return

    const current = execution.value
    if (!current || current.id !== event.execution_id) {
      if (current) supersededExecutionIds.add(current.id)
      execution.value = {
        id: event.execution_id,
        workflow_id: event.workflow_id,
        status: 'Running',
        mode: 'Manual',
        node_outputs: {},
        started_at: new Date().toISOString(),
        finished_at: null,
      }
      if (event.type !== 'execution_finished') void catchUp(event.execution_id)
    }

    const updated = execution.value!
    const runs = (updated.node_runs ??= {})
    switch (event.type) {
      case 'node_finished':
      case 'node_skipped':
        updated.node_outputs[event.node_id] = event.items
        runs[event.node_id] = {
          status: event.type === 'node_finished' ? 'success' : 'skipped',
          counts: event.counts ?? { '0': event.items.length },
          outputs: [event.items],
          ...(event.type === 'node_finished' && event.reused ? { reused: true } : {}),
        }
        break
      case 'node_errored':
        updated.node_outputs[event.node_id] = [{ json: { error: event.error }, binary: {} }]
        runs[event.node_id] = { status: 'error', counts: event.counts ?? {}, error: event.error }
        break
      case 'execution_finished':
        updated.status = event.status
        updated.finished_at = new Date().toISOString()
        void catchUp(event.execution_id)
        break
      case 'node_started':
        runs[event.node_id] = { status: 'running', counts: {} }
        break
    }
  }

  function connect() {
    const token = getToken()
    if (!token) return
    wanted = true
    const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:'
    const ws = new WebSocket(`${protocol}//${location.host}/ws/workflows/${workflowId}/executions`)
    socket = ws
    ws.addEventListener('open', () => ws.send(JSON.stringify({ token })))
    // Dropped (server restarted, network blip): reconnect, or later runs
    // never show. A run in progress then catches up on its next event.
    ws.addEventListener('close', () => {
      if (!wanted || socket !== ws) return
      socket = null
      reconnectTimer = setTimeout(() => {
        reconnectTimer = null
        if (wanted) connect()
      }, RECONNECT_MS)
    })
    ws.addEventListener('message', (e) => {
      try {
        applyEvent(JSON.parse((e as MessageEvent).data as string) as LiveExecutionEvent)
      } catch {
        // Malformed frame -- ignore. useWorkflowRun's polling fallback
        // still resolves the run from GET /rest/executions/:id.
      }
    })
  }

  function disconnect() {
    wanted = false
    if (reconnectTimer) {
      clearTimeout(reconnectTimer)
      reconnectTimer = null
    }
    socket?.close()
    socket = null
  }

  return { execution, connect, disconnect }
}
