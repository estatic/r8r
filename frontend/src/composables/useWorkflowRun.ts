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
  // The execute request itself, while the server is still starting the run
  // (a Telegram trigger waits there for its test message).
  let starting: AbortController | null = null
  let timer: ReturnType<typeof setInterval> | null = null
  let disposed = false

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
      // The socket then swaps in the server's final copy itself.
      if (startedId && e?.id === startedId && e.status !== 'Running') stop()
    },
    { deep: true },
  )

  /** Unchanged nodes that succeeded last time are reused unless `fresh`. */
  async function execute(fresh = false) {
    executing.value = true
    let started: Execution
    const controller = new AbortController()
    starting = controller
    try {
      const url = `/rest/r8r/workflows/${workflowId}/execute${fresh ? '?fresh=true' : ''}`
      started = await api.post<Execution>(url, undefined, { signal: controller.signal })
    } catch (e) {
      stop()
      if (controller.signal.aborted) return // stopped by the user
      throw e
    } finally {
      if (starting === controller) starting = null
    }
    // The editor was left while the POST was in flight: nothing to track.
    if (disposed) {
      stop()
      return
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
        const latest = await api.get<Execution>(`/rest/r8r/executions/${id}`)
        if (startedId === id && latest.status !== 'Running') {
          stop()
          execution.value = latest
        }
      } catch {
        // Transient; the next tick retries.
      }
    }, pollMs)
  }

  /**
   * Stops the run: a started one on the server (it then ends as Canceled
   * over the socket or the poll), the execute request still waiting for
   * the run to start, or a run on screen that something else started
   * (a trigger).
   */
  async function cancel() {
    const running = execution.value?.status === 'Running' ? execution.value.id : null
    if (startedId ?? running) {
      await api.post(`/rest/r8r/executions/${startedId ?? running}/stop`)
    } else if (starting) {
      starting.abort()
    }
  }

  if (getCurrentScope()) {
    onScopeDispose(() => {
      disposed = true
      stop()
    })
  }

  return { executing, execute, stop, cancel }
}
