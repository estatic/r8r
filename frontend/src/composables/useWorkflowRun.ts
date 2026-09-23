import { getCurrentScope, onScopeDispose, ref, watch, type Ref } from 'vue'
import { api } from '../api/client'
import type { Execution } from '../types/domain'

/**
 * Starts a manual run (POST .../execute answers 202 with the Running
 * execution, Plan 8.7) and tracks it until it finishes. `execution` is the
 * live socket's ref: socket events normally complete the run; a GET poll
 * every `pollMs` is the backstop for a dropped or missing socket.
 */
export function useWorkflowRun(workflowId: string, execution: Ref<Execution | null>, pollMs = 2000) {
  const executing = ref(false)
  let startedId: string | null = null
  let timer: ReturnType<typeof setInterval> | null = null

  function stop() {
    executing.value = false
    startedId = null
    if (timer) {
      clearInterval(timer)
      timer = null
    }
  }

  // The socket mutates the execution object in place, hence deep.
  watch(
    execution,
    (e) => {
      if (startedId && e?.id === startedId && e.status !== 'Running') stop()
    },
    { deep: true },
  )

  async function execute() {
    executing.value = true
    let started: Execution
    try {
      started = await api.post<Execution>(`/rest/workflows/${workflowId}/execute`)
    } catch (e) {
      stop()
      throw e
    }
    // Socket events for this run may have landed first: keep that richer copy.
    if (execution.value?.id !== started.id) execution.value = started
    if (execution.value!.status !== 'Running') {
      stop()
      return
    }
    startedId = started.id
    const id = started.id
    timer = setInterval(async () => {
      try {
        const latest = await api.get<Execution>(`/rest/executions/${id}`)
        if (startedId === id && latest.status !== 'Running') execution.value = latest
      } catch {
        // Transient; the next tick retries.
      }
    }, pollMs)
  }

  if (getCurrentScope()) onScopeDispose(stop)

  return { executing, execute, stop }
}