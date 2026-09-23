import { ref, type Ref } from 'vue'
import { getToken } from '../api/client'
import type { Execution, ExecutionStatus, Item } from '../types/domain'

export type LiveExecutionEvent = { execution_id: string; workflow_id: string } & (
  | { type: 'node_started'; node_id: string }
  | { type: 'node_finished'; node_id: string; items: Item[] }
  | { type: 'node_errored'; node_id: string; error: string }
  | { type: 'node_skipped'; node_id: string; items: Item[] }
  | { type: 'execution_finished'; status: ExecutionStatus }
)

export interface LiveExecutionSocket {
  execution: Ref<Execution | null>
  connect: () => void
  disconnect: () => void
}

export function useLiveExecutionSocket(workflowId: string): LiveExecutionSocket {
  const execution = ref<Execution | null>(null)
  let socket: WebSocket | null = null
  const supersededExecutionIds = new Set<string>()

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
    }

    const updated = execution.value!
    switch (event.type) {
      case 'node_finished':
      case 'node_skipped':
        updated.node_outputs[event.node_id] = event.items
        break
      case 'node_errored':
        updated.node_outputs[event.node_id] = [{ json: { error: event.error }, binary: {} }]
        break
      case 'execution_finished':
        updated.status = event.status
        updated.finished_at = new Date().toISOString()
        break
      case 'node_started':
        break
    }
  }

  function connect() {
    const token = getToken()
    if (!token) return
    const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:'
    socket = new WebSocket(`${protocol}//${location.host}/ws/workflows/${workflowId}/executions`)
    socket.addEventListener('open', () => socket?.send(JSON.stringify({ token })))
    socket.addEventListener('message', (e) => {
      try {
        applyEvent(JSON.parse((e as MessageEvent).data as string) as LiveExecutionEvent)
      } catch {
        // Malformed frame -- ignore. useWorkflowRun's polling fallback
        // still resolves the run from GET /rest/executions/:id.
      }
    })
  }

  function disconnect() {
    socket?.close()
    socket = null
  }

  return { execution, connect, disconnect }
}
