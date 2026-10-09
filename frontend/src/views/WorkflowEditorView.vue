<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import { api, ApiError } from '../api/client'
import { errorText } from '../api/errorText'
import { useExecutionsStore } from '../stores/executions'
import { useLiveExecutionSocket } from '../composables/useLiveExecutionSocket'
import { useWorkflowRun } from '../composables/useWorkflowRun'
import { agentSetupProblems, describeSetupProblems } from '../agent/setup'
import { useUnsavedChanges } from '../composables/useUnsavedChanges'
import type { Workflow, Connection, NodeInstance, Execution } from '../types/domain'
import WorkflowCanvas from '../components/WorkflowCanvas.vue'
import AddNodeMenu from '../components/AddNodeMenu.vue'
import NodeConfigPanel from '../components/NodeConfigPanel.vue'
import ExecutionResultsPanel from '../components/ExecutionResultsPanel.vue'
import NodeDetailsView from '../components/NodeDetailsView.vue'
import { insertNodeIntoConnection, removeConnection, removeNode } from '../canvas/edit'
import { upstreamSources } from '../canvas/inputData'
import { useNodeTypesStore } from '../stores/nodeTypes'

const route = useRoute()
const workflowId = route.params.id as string
const executionsStore = useExecutionsStore()

const workflow = ref<Workflow | null>(null)
const changes = useUnsavedChanges(workflow)
const selectedNodeId = ref<string | null>(null)
const saving = ref(false)
const loadingHistory = ref(false)
const live = useLiveExecutionSocket(workflowId)
const execution = live.execution
const run = useWorkflowRun(workflowId, execution)
const executing = run.executing
const loadError = ref('')
const actionError = ref('')

const selectedNode = computed<NodeInstance | null>(
  () => workflow.value?.nodes.find((n) => n.id === selectedNodeId.value) ?? null,
)

/**
 * `crypto.randomUUID()` only exists in a secure context (https, localhost).
 * r8r is self-hosted and commonly reached at `http://<lan-ip>:3000`, where it
 * is `undefined` — so fall back to a non-cryptographic id. These ids are
 * workflow-scoped local identifiers, not secrets.
 */
function newNodeId(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID()
  }
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2)}`
}

function messageFor(e: unknown, fallback: string): string {
  return errorText(e, fallback)
}

onMounted(async () => {
  live.connect()
  try {
    workflow.value = await api.get<Workflow>(`/rest/r8r/workflows/${workflowId}`)
    changes.markSaved()
  } catch (e) {
    loadError.value = messageFor(e, 'Failed to load workflow.')
  }
})

onUnmounted(() => {
  live.disconnect()
  window.removeEventListener('keydown', onKeydown)
})

// What reaches the selected node, for pickers like the Set node's: from the
// run on screen, else the latest run (fetched when a node is opened).
const latestRun = ref<Execution | null>(null)
watch(selectedNodeId, async (id) => {
  if (!id || execution.value || latestRun.value) return
  try {
    latestRun.value = (await api.get<Execution[]>(`/rest/r8r/workflows/${workflowId}/executions?limit=1`))[0] ?? null
  } catch {
    // The picker then just explains there's no data yet.
  }
})
const inputSources = computed(() =>
  workflow.value && selectedNodeId.value ? upstreamSources(workflow.value, selectedNodeId.value, execution.value ?? latestRun.value) : [],
)
const nodeTypesStore = useNodeTypesStore()
const nodeLabels = computed<Record<string, string>>(() =>
  Object.fromEntries(
    (workflow.value?.nodes ?? []).map((n) => {
      const meta = nodeTypesStore.types.find((t) => t.type_name === n.node_type)
      return [n.id, meta ? `${meta.icon} ${meta.display_name}` : n.node_type]
    }),
  ),
)

function deleteNode(nodeId: string) {
  if (!workflow.value) return
  removeNode(workflow.value, nodeId)
  if (selectedNodeId.value === nodeId) selectedNodeId.value = null
}

// Delete / Backspace removes the selected node, unless the user is typing.
function onKeydown(e: KeyboardEvent) {
  if (e.key !== 'Delete' && e.key !== 'Backspace') return
  const el = document.activeElement
  if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement || (el as HTMLElement | null)?.isContentEditable) return
  if (selectedNodeId.value) {
    e.preventDefault()
    deleteNode(selectedNodeId.value)
  }
}
window.addEventListener('keydown', onKeydown)

function onEdgeDelete(connection: Connection) {
  if (workflow.value) removeConnection(workflow.value, connection)
}

// The link a node is being put into, while the node menu is open for it.
const insertInto = ref<Connection | null>(null)
const insertMenuOpen = computed({
  get: () => insertInto.value !== null,
  set: (open: boolean) => {
    if (!open) insertInto.value = null
  },
})

function onEdgeInsert(connection: Connection) {
  insertInto.value = connection
}

function onInsertNode(nodeType: string) {
  if (!workflow.value || !insertInto.value) return
  const node: NodeInstance = { id: newNodeId(), node_type: nodeType, position: [0, 0], parameters: {}, disabled: false }
  insertNodeIntoConnection(workflow.value, insertInto.value, node)
  insertInto.value = null
  selectedNodeId.value = node.id
}

// The open node panel. Its edits apply by themselves after a short pause;
// flushPanel() applies any still pending before anything reads or leaves
// the workflow, so nothing typed is lost. False: an edit is invalid (the
// panel shows why), so the caller stops.
const panel = ref<{ flush: () => boolean } | null>(null)
const canvas = ref<{ freeSpot: () => [number, number] } | null>(null)
function flushPanel(): boolean {
  const ok = panel.value?.flush() ?? true
  if (!ok) actionError.value = 'Fix the node settings shown in the panel first.'
  return ok
}

function onNodeSelect(nodeId: string) {
  if (nodeId !== selectedNodeId.value && !flushPanel()) return
  selectedNodeId.value = nodeId
}

// An agent port was clicked: open the agent with that part of its settings in view.
const panelFocus = ref<{ section: string; at: number } | null>(null)
function onAuxOpen(nodeId: string, port: string) {
  if (nodeId !== selectedNodeId.value && !flushPanel()) return
  selectedNodeId.value = nodeId
  panelFocus.value = { section: port, at: Date.now() }
}

function closePanel() {
  if (flushPanel()) selectedNodeId.value = null
}

function onNodeMove(nodeId: string, position: [number, number]) {
  if (!workflow.value) return
  const node = workflow.value.nodes.find((n) => n.id === nodeId)
  if (node) node.position = position
}

function onConnect(connection: Connection) {
  if (!workflow.value) return
  workflow.value.connections.push(connection)
}

function onAddNode(nodeType: string) {
  if (!workflow.value) return
  workflow.value.nodes.push({
    id: newNodeId(),
    node_type: nodeType,
    // In the middle of what is on screen, clear of the nodes already there.
    position: canvas.value?.freeSpot() ?? [100 + workflow.value.nodes.length * 240, 100],
    parameters: {},
    disabled: false,
  })
}

function onNodeUpdate(updated: NodeInstance) {
  if (!workflow.value) return
  const idx = workflow.value.nodes.findIndex((n) => n.id === updated.id)
  if (idx !== -1) workflow.value.nodes[idx] = updated
}

/** Returns whether the workflow was saved. */
async function save(): Promise<boolean> {
  if (!workflow.value) return false
  actionError.value = ''
  if (!flushPanel()) return false
  saving.value = true
  try {
    workflow.value = await api.put<Workflow>(`/rest/r8r/workflows/${workflowId}`, {
      name: workflow.value.name,
      nodes: workflow.value.nodes,
      connections: workflow.value.connections,
    })
    // Saving an unfinished agent is fine (work in progress), but say so now
    // rather than at the next run.
    changes.markSaved()
    const problems = agentSetupProblems(workflow.value.nodes)
    if (problems.length > 0) actionError.value = `Saved. ${describeSetupProblems(problems)}`
    return true
  } catch (e) {
    // Leave workflow.value untouched so the user keeps their unsaved edits
    // and can simply retry.
    actionError.value = messageFor(e, 'Failed to save workflow — your changes were not saved.')
    return false
  } finally {
    saving.value = false
  }
}

/** Runs the workflow; nodes that succeeded last time and haven't changed are reused unless `fresh`. */
async function execute(fresh = false) {
  actionError.value = ''
  if (!flushPanel()) return
  const problems = workflow.value ? agentSetupProblems(workflow.value.nodes) : []
  if (problems.length > 0) {
    actionError.value = describeSetupProblems(problems)
    return
  }
  // Execute runs the saved workflow on the server: save pending edits
  // (the open panel's were applied above) first, so what runs is what's on screen.
  if (changes.dirty.value && !(await save())) return
  try {
    await run.execute(fresh)
  } catch (e) {
    actionError.value = messageFor(e, 'Failed to execute workflow.')
  }
}

// The run on screen reused every node: nothing ran (say so, or it looks like nothing happened).
const allReused = computed(() => {
  const e = execution.value
  if (!e || e.status === 'Running') return false
  const runs = Object.values(e.node_runs ?? {})
  return runs.length > 0 && runs.every((r) => r.reused)
})

const activating = ref(false)

/**
 * Turns the workflow's trigger on or off. While on, the trigger runs it by
 * itself (each Telegram message, webhook call, schedule tick) and every run
 * shows live on the canvas. The server activates the saved workflow, so
 * pending edits are saved first.
 */
async function toggleActive() {
  if (!workflow.value) return
  actionError.value = ''
  const active = !workflow.value.active
  if (!flushPanel()) return
  if (active && changes.dirty.value && !(await save())) return
  activating.value = true
  try {
    const updated = await api.patch<Workflow>(`/rest/r8r/workflows/${workflowId}/active`, { active })
    workflow.value.active = updated.active
  } catch (e) {
    actionError.value = messageFor(e, active ? 'Failed to activate the workflow.' : 'Failed to deactivate the workflow.')
  } finally {
    activating.value = false
  }
}

async function stopRun() {
  actionError.value = ''
  try {
    await run.cancel()
  } catch (e) {
    actionError.value = messageFor(e, 'Failed to stop the execution.')
  }
}

async function showHistory() {
  actionError.value = ''
  loadingHistory.value = true
  try {
    await executionsStore.fetchHistory(workflowId)
    if (!execution.value && executionsStore.history.length > 0) {
      execution.value = executionsStore.history[0]
    }
  } catch (e) {
    actionError.value = messageFor(e, 'Failed to load execution history.')
  } finally {
    loadingHistory.value = false
  }
}
</script>

<template>
  <div class="h-screen flex flex-col">
    <header class="bg-white border-b px-6 py-3 flex items-center gap-4">
      <router-link to="/workflows" class="text-sm text-gray-500">&larr; Workflows</router-link>
      <input
        v-if="workflow"
        v-model="workflow.name"
        class="text-lg font-medium border-none focus:outline-none focus:ring-1 focus:ring-blue-300 rounded px-1"
      />
      <div class="flex-1"></div>
      <button
        v-if="workflow"
        type="button"
        role="switch"
        data-testid="active-toggle"
        :aria-checked="workflow.active"
        :disabled="activating"
        :title="workflow.active ? 'Its trigger runs it by itself; click to stop' : 'Let its trigger run it by itself'"
        class="flex items-center gap-2 text-sm disabled:opacity-50"
        @click="toggleActive"
      >
        <span class="relative inline-block w-9 h-5 rounded-full transition-colors" :class="workflow.active ? 'bg-green-600' : 'bg-gray-300'">
          <span class="absolute top-0.5 w-4 h-4 rounded-full bg-white shadow transition-all" :class="workflow.active ? 'left-[18px]' : 'left-0.5'" />
        </span>
        <span :class="workflow.active ? 'text-green-700 font-medium' : 'text-gray-600'">{{ workflow.active ? 'Active' : 'Inactive' }}</span>
      </button>
      <AddNodeMenu @add="onAddNode" />
      <button
        class="bg-gray-200 text-gray-800 rounded px-3 py-1.5 text-sm disabled:opacity-50"
        :disabled="saving"
        @click="save"
      >
        {{ saving ? 'Saving…' : 'Save' }}
      </button>
      <span v-if="changes.dirty.value && !saving" data-testid="unsaved" class="text-xs text-amber-700">Unsaved changes</span>
      <button
        class="bg-green-600 text-white rounded px-3 py-1.5 text-sm disabled:opacity-50"
        :disabled="executing"
        data-testid="execute"
        title="Runs the nodes that failed, changed or haven't run; the others keep their last output"
        @click="execute(false)"
      >
        {{ executing ? 'Running…' : 'Execute' }}
      </button>
      <button
        v-if="!executing"
        class="bg-gray-200 text-gray-800 rounded px-3 py-1.5 text-sm"
        data-testid="execute-fresh"
        title="Runs every node again (a Telegram trigger waits for a new message)"
        @click="execute(true)"
      >
        Run all again
      </button>
      <button
        v-if="executing"
        data-testid="stop-execution"
        class="bg-red-600 text-white rounded px-3 py-1.5 text-sm"
        @click="stopRun"
      >
        Stop
      </button>
      <button
        class="bg-gray-200 text-gray-800 rounded px-3 py-1.5 text-sm disabled:opacity-50"
        :disabled="loadingHistory"
        @click="showHistory"
      >
        {{ loadingHistory ? 'Loading…' : 'History' }}
      </button>
    </header>
    <p v-if="actionError" class="bg-red-50 border-b border-red-200 px-6 py-2 text-sm text-red-600">{{ actionError }}</p>
    <p v-if="allReused" data-testid="all-reused" class="bg-blue-50 border-b border-blue-200 px-6 py-2 text-sm text-blue-800">
      Nothing changed since the last run, so every node kept its output. Use <b>Run all again</b> to run everything.
    </p>
    <div class="flex-1 relative">
      <p v-if="loadError" class="p-6 text-sm text-red-600">{{ loadError }}</p>
      <WorkflowCanvas
        v-if="workflow"
        ref="canvas"
        :nodes="workflow.nodes"
        :connections="workflow.connections"
        :execution="execution"
        @edge-insert="onEdgeInsert"
        @aux-open="onAuxOpen"
        @edge-delete="onEdgeDelete"
        @node-select="onNodeSelect"
        @node-move="onNodeMove"
        @connect="onConnect"
      />
      <NodeDetailsView v-if="selectedNode && workflow" :node="selectedNode" :workflow="workflow" :execution="execution ?? latestRun" @close="closePanel">
        <NodeConfigPanel
          ref="panel"
          embedded
          :workflow-id="workflow?.id"
          :node="selectedNode"
          :focus="panelFocus"
          :input-sources="inputSources"
          :node-labels="nodeLabels"
          @update="onNodeUpdate" @delete="deleteNode" @close="closePanel" />
      </NodeDetailsView>
      <div v-if="insertInto" data-testid="insert-menu" class="absolute top-4 left-1/2 -translate-x-1/2 z-20">
        <p class="text-xs text-gray-600 bg-white border rounded px-2 py-1 mb-1 shadow">Choose the node to put into this connection</p>
        <AddNodeMenu v-model:open="insertMenuOpen" hide-button @add="onInsertNode" />
      </div>
      <ExecutionResultsPanel
        :execution="execution"
        :history="executionsStore.history"
        @close="execution = null"
        @select="execution = $event"
      />
    </div>
  </div>
</template>
