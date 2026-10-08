import { computed, getCurrentScope, onScopeDispose, ref, type Ref } from 'vue'
import type { Workflow } from '../types/domain'

/** What Save persists: everything except server-managed fields. */
function snapshot(workflow: Workflow | null): string {
  if (!workflow) return ''
  return JSON.stringify({ name: workflow.name, nodes: workflow.nodes, connections: workflow.connections })
}

/**
 * Tracks whether the editor's workflow differs from the last saved copy.
 * Execute runs the *saved* workflow on the server, so the editor uses this
 * to save first instead of running a stale version, and the browser asks
 * before a reload or close discards unsaved edits.
 */
export function useUnsavedChanges(workflow: Ref<Workflow | null>) {
  const saved = ref(snapshot(workflow.value))
  const dirty = computed(() => snapshot(workflow.value) !== saved.value)

  function markSaved() {
    saved.value = snapshot(workflow.value)
  }

  // Node "Apply" only edits this copy: warn before a reload or close
  // throws unsaved edits away.
  function onBeforeUnload(event: BeforeUnloadEvent) {
    if (!dirty.value) return
    event.preventDefault()
    event.returnValue = ''
  }
  if (getCurrentScope()) {
    window.addEventListener('beforeunload', onBeforeUnload)
    onScopeDispose(() => window.removeEventListener('beforeunload', onBeforeUnload))
  }

  return { dirty, markSaved }
}
