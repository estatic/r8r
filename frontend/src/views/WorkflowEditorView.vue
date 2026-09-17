<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { useRoute } from 'vue-router'
import { api } from '../api/client'
import type { Workflow, Connection } from '../types/domain'
import WorkflowCanvas from '../components/WorkflowCanvas.vue'

const route = useRoute()
const workflowId = route.params.id as string

const workflow = ref<Workflow | null>(null)
const selectedNodeId = ref<string | null>(null)

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
    </div>
  </div>
</template>
