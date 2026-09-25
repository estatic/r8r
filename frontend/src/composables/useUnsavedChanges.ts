import { computed, ref, type Ref } from 'vue'
import type { Workflow } from '../types/domain'

/** What Save persists: everything except server-managed fields. */
function snapshot(workflow: Workflow | null): string {
  if (!workflow) return ''
  return JSON.stringify({ name: workflow.name, nodes: workflow.nodes, connections: workflow.connections })
}

/**
 * Tracks whether the editor's workflow differs from the last saved copy.
 * Execute runs the *saved* workflow on the server, so the editor uses this
 * to save first instead of running a stale version.
 */
export function useUnsavedChanges(workflow: Ref<Workflow | null>) {
  const saved = ref(snapshot(workflow.value))
  const dirty = computed(() => snapshot(workflow.value) !== saved.value)

  function markSaved() {
    saved.value = snapshot(workflow.value)
  }

  return { dirty, markSaved }
}
