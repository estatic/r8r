<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRoute } from 'vue-router'
import { api, ApiError } from '../api/client'
import type { Workflow, Connection, NodeInstance, Execution } from '../types/domain'
import WorkflowCanvas from '../components/WorkflowCanvas.vue'
import AddNodeMenu from '../components/AddNodeMenu.vue'
import NodeConfigPanel from '../components/NodeConfigPanel.vue'
import ExecutionResultsPanel from '../components/ExecutionResultsPanel.vue'

const route = useRoute()
const workflowId = route.params.id as string

const workflow = ref<Workflow | null>(null)
const selectedNodeId = ref<string | null>(null)
const saving = ref(false)
const executing = ref(false)
const execution = ref<Execution | null>(null)
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
  return e instanceof ApiError ? `${fallback} (${e.status})` : fallback
}

onMounted(async () => {
  try {
    workflow.value = await api.get<Workflow>(`/rest/workflows/${workflowId}`)
  } catch (e) {
    loadError.value = messageFor(e, 'Failed to load workflow.')
  }
})

function onNodeSelect(nodeId: string) {
  selectedNodeId.value = nodeId
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
  const count = workflow.value.nodes.length
  workflow.value.nodes.push({
    id: newNodeId(),
    node_type: nodeType,
    // Stack new nodes vertically with enough clearance that a node never
    // covers the one above it: nodes are ~100px tall and the connection
    // handles sit on their top and bottom edges, so an overlapping node would
    // swallow the pointer events a connection drag has to start from.
    position: [100, 100 + count * 160],
    parameters: {},
    disabled: false,
  })
}

function onNodeUpdate(updated: NodeInstance) {
  if (!workflow.value) return
  const idx = workflow.value.nodes.findIndex((n) => n.id === updated.id)
  if (idx !== -1) workflow.value.nodes[idx] = updated
}

async function save() {
  if (!workflow.value) return
  actionError.value = ''
  saving.value = true
  try {
    workflow.value = await api.put<Workflow>(`/rest/workflows/${workflowId}`, {
      name: workflow.value.name,
      nodes: workflow.value.nodes,
      connections: workflow.value.connections,
    })
  } catch (e) {
    // Leave workflow.value untouched so the user keeps their unsaved edits
    // and can simply retry.
    actionError.value = messageFor(e, 'Failed to save workflow — your changes were not saved.')
  } finally {
    saving.value = false
  }
}

async function execute() {
  actionError.value = ''
  executing.value = true
  try {
    execution.value = await api.post<Execution>(`/rest/workflows/${workflowId}/execute`)
  } catch (e) {
    actionError.value = messageFor(e, 'Failed to execute workflow.')
  } finally {
    executing.value = false
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
      <AddNodeMenu @add="onAddNode" />
      <button
        class="bg-gray-200 text-gray-800 rounded px-3 py-1.5 text-sm disabled:opacity-50"
        :disabled="saving"
        @click="save"
      >
        {{ saving ? 'Saving…' : 'Save' }}
      </button>
      <button
        class="bg-green-600 text-white rounded px-3 py-1.5 text-sm disabled:opacity-50"
        :disabled="executing"
        @click="execute"
      >
        {{ executing ? 'Running…' : 'Execute' }}
      </button>
    </header>
    <p v-if="actionError" class="bg-red-50 border-b border-red-200 px-6 py-2 text-sm text-red-600">{{ actionError }}</p>
    <div class="flex-1 relative">
      <p v-if="loadError" class="p-6 text-sm text-red-600">{{ loadError }}</p>
      <WorkflowCanvas
        v-if="workflow"
        :nodes="workflow.nodes"
        :connections="workflow.connections"
        @node-select="onNodeSelect"
        @node-move="onNodeMove"
        @connect="onConnect"
      />
      <NodeConfigPanel :node="selectedNode" @update="onNodeUpdate" @close="selectedNodeId = null" />
      <ExecutionResultsPanel :execution="execution" @close="execution = null" />
    </div>
  </div>
</template>
