<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRoute } from 'vue-router'
import { api } from '../api/client'
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

const selectedNode = computed<NodeInstance | null>(
  () => workflow.value?.nodes.find((n) => n.id === selectedNodeId.value) ?? null,
)

onMounted(async () => {
  workflow.value = await api.get<Workflow>(`/rest/workflows/${workflowId}`)
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
    id: crypto.randomUUID(),
    node_type: nodeType,
    position: [100 + count * 40, 100 + count * 40],
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
  saving.value = true
  try {
    workflow.value = await api.put<Workflow>(`/rest/workflows/${workflowId}`, {
      name: workflow.value.name,
      nodes: workflow.value.nodes,
      connections: workflow.value.connections,
    })
  } finally {
    saving.value = false
  }
}

async function execute() {
  executing.value = true
  try {
    execution.value = await api.post<Execution>(`/rest/workflows/${workflowId}/execute`)
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
    <div class="flex-1 relative">
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
