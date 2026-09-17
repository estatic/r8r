<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRoute } from 'vue-router'
import { api } from '../api/client'
import type { Workflow, Connection, NodeInstance } from '../types/domain'
import WorkflowCanvas from '../components/WorkflowCanvas.vue'
import AddNodeMenu from '../components/AddNodeMenu.vue'
import NodeConfigPanel from '../components/NodeConfigPanel.vue'

const route = useRoute()
const workflowId = route.params.id as string

const workflow = ref<Workflow | null>(null)
const selectedNodeId = ref<string | null>(null)

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
    </div>
  </div>
</template>
