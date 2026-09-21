<script setup lang="ts">
import { computed, reactive, watch } from 'vue'
import { VueFlow, Handle, Position, MarkerType, useVueFlow, type Node as FlowNode, type Edge as FlowEdge } from '@vue-flow/core'
import '@vue-flow/core/dist/style.css'
import type { NodeInstance, Connection } from '../types/domain'
import { useNodeTypesStore } from '../stores/nodeTypes'

const props = defineProps<{
  nodes: NodeInstance[]
  connections: Connection[]
}>()

const emit = defineEmits<{
  'node-select': [nodeId: string]
  'node-move': [nodeId: string, position: [number, number]]
  connect: [connection: Connection]
}>()

const { onConnect, onNodeDragStop, onNodeClick } = useVueFlow()

const nodeTypesStore = useNodeTypesStore()
if (!nodeTypesStore.loaded) {
  nodeTypesStore.fetchAll().catch(() => {})
}

function labelFor(nodeType: string): string {
  const meta = nodeTypesStore.types.find((t) => t.type_name === nodeType)
  return meta ? `${meta.icon} ${meta.display_name}` : nodeType
}

// nodeId -> live output port labels for that node's current parameters.
// Populated by watching props.nodes and re-fetching only when a given
// node's (node_type, parameters) pair actually changes -- not per
// keystroke, since NodeConfigPanel only propagates parameter edits on
// Apply.
const portsByNodeId = reactive<Record<string, string[]>>({})
const lastFetchedKey = new Map<string, string>()

watch(
  () => props.nodes,
  (nodes) => {
    for (const n of nodes) {
      const key = `${n.node_type}:${JSON.stringify(n.parameters)}`
      if (lastFetchedKey.get(n.id) === key) continue
      lastFetchedKey.set(n.id, key)
      nodeTypesStore
        .portsFor(n.node_type, n.parameters)
        .then((ports) => {
          portsByNodeId[n.id] = ports
        })
        .catch(() => {
          portsByNodeId[n.id] = ['main']
        })
    }
  },
  { immediate: true, deep: true },
)

const flowNodes = computed<FlowNode[]>(() =>
  props.nodes.map((n) => ({
    id: n.id,
    position: { x: n.position[0], y: n.position[1] },
    label: labelFor(n.node_type),
    data: { nodeType: n.node_type, disabled: n.disabled, outputPorts: portsByNodeId[n.id] ?? ['main'] },
  })),
)

const flowEdges = computed<FlowEdge[]>(() =>
  props.connections.map((c) => ({
    id: `${c.from_node}:${c.error ? 'error' : c.from_output}->${c.to_node}:${c.to_input}`,
    source: c.from_node,
    target: c.to_node,
    sourceHandle: c.error ? 'error' : String(c.from_output),
    targetHandle: String(c.to_input),
    markerEnd: MarkerType.ArrowClosed,
    style: c.error ? { stroke: '#dc2626' } : undefined,
  })),
)

onNodeClick((event) => {
  emit('node-select', event.node.id)
})

onNodeDragStop((event) => {
  emit('node-move', event.node.id, [event.node.position.x, event.node.position.y])
})

onConnect((connection) => {
  const isError = connection.sourceHandle === 'error'
  emit('connect', {
    from_node: connection.source,
    from_output: isError ? 0 : Number(connection.sourceHandle ?? 0),
    to_node: connection.target,
    to_input: Number(connection.targetHandle ?? 0),
    error: isError,
  })
})

function handlePosition(index: number, total: number): string {
  return `${((index + 1) * 100) / (total + 1)}%`
}
</script>

<template>
  <div class="w-full h-full">
    <VueFlow :nodes="flowNodes" :edges="flowEdges" fit-view-on-init :delete-key-code="null">
      <template #node-default="{ data, label }">
        <Handle id="0" type="target" :position="Position.Top" class="w-2.5 h-2.5 rounded-full bg-gray-500 border border-white" />
        <div
          class="px-3 py-2 rounded border bg-white shadow text-xs"
          :class="{ 'opacity-50': data.disabled }"
          :title="data.nodeType"
        >
          {{ label }}
        </div>
        <Handle
          v-for="(port, i) in data.outputPorts"
          :key="port"
          :id="String(i)"
          type="source"
          :position="Position.Bottom"
          :style="{ left: handlePosition(i, data.outputPorts.length + 1) }"
          class="w-2.5 h-2.5 rounded-full bg-gray-500 border border-white"
        />
        <div
          v-for="(port, i) in data.outputPorts"
          :key="`label-${port}`"
          class="absolute text-[10px] text-gray-600 top-full mt-0.5"
          :style="{ left: handlePosition(i, data.outputPorts.length + 1), transform: 'translateX(-50%)' }"
        >
          {{ port }}
        </div>
        <Handle
          id="error"
          type="source"
          :position="Position.Bottom"
          :style="{ left: handlePosition(data.outputPorts.length, data.outputPorts.length + 1) }"
          class="w-2.5 h-2.5 rounded-full bg-red-500 border border-white"
        />
        <div
          class="absolute text-[10px] text-red-600 top-full mt-0.5"
          :style="{ left: handlePosition(data.outputPorts.length, data.outputPorts.length + 1), transform: 'translateX(-50%)' }"
        >
          error
        </div>
      </template>
    </VueFlow>
  </div>
</template>
