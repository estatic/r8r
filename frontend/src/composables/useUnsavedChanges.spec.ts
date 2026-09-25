import { describe, it, expect } from 'vitest'
import { nextTick, ref } from 'vue'
import { useUnsavedChanges } from './useUnsavedChanges'
import type { Workflow } from '../types/domain'

function wf(model: string): Workflow {
  return {
    id: 'w',
    name: 'test',
    active: false,
    nodes: [{ id: 'agent', node_type: 'ai.agent', position: [0, 0], parameters: { model }, disabled: false }],
    connections: [],
    created_at: '',
    updated_at: '',
  } as Workflow
}

describe('useUnsavedChanges', () => {
  it('is clean after marking the loaded workflow as saved', async () => {
    const workflow = ref<Workflow | null>(wf('a'))
    const changes = useUnsavedChanges(workflow)
    changes.markSaved()
    await nextTick()
    expect(changes.dirty.value).toBe(false)
  })

  it('becomes dirty when a node is changed (e.g. Apply in the node panel)', async () => {
    const workflow = ref<Workflow | null>(wf('a'))
    const changes = useUnsavedChanges(workflow)
    changes.markSaved()
    workflow.value!.nodes[0] = { ...workflow.value!.nodes[0], parameters: { model: 'qwen' } }
    await nextTick()
    expect(changes.dirty.value).toBe(true)
  })

  it('is clean again after saving', async () => {
    const workflow = ref<Workflow | null>(wf('a'))
    const changes = useUnsavedChanges(workflow)
    changes.markSaved()
    workflow.value!.name = 'renamed'
    await nextTick()
    changes.markSaved()
    await nextTick()
    expect(changes.dirty.value).toBe(false)
  })

  it('ignores server-managed timestamps', async () => {
    const workflow = ref<Workflow | null>(wf('a'))
    const changes = useUnsavedChanges(workflow)
    changes.markSaved()
    workflow.value = { ...workflow.value!, updated_at: 'later' }
    await nextTick()
    expect(changes.dirty.value).toBe(false)
  })
})
