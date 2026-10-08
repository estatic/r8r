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

  // The socket finished the run, but it may have missed node events (e.g.
  // the results panel was closed mid-run): show the server's final copy.
  function finishFromSocket(id: string) {
    stop()
    api
      .get<Execution>(`/rest/r8r/executions/${id}`)
      .then((latest) => {
        if (!disposed && latest.status !== 'Running' && execution.value?.id === id) execution.value = latest
      })
      .catch(() => {
        // Keep the socket's copy.
      })
  }

  // The socket mutates the execution object in place, hence deep.
  watch(
    execution,
    (e) => {
      if (startedId && e?.id === startedId && e.status !== 'Running') finishFromSocket(startedId)
    },
    { deep: true },
  )

  async function execute() {
    executing.value = true
    let started: Execution
    const controller = new AbortController()
    starting = controller
    try {
      started = await api.post<Execution>(`/rest/r8r/workflows/${workflowId}/execute`, undefined, { signal: controller.signal })
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
      finishFromSocket(started.id)
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
   * over the socket or the poll), or the execute request still waiting
   * for the run to start.
   */
  async function cancel() {
    if (startedId) {
      await api.post(`/rest/r8r/executions/${startedId}/stop`)
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
