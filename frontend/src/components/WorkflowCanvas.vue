<script setup lang="ts">
import { computed } from 'vue'
import { VueFlow, useVueFlow, type Node as FlowNode, type Edge as FlowEdge } from '@vue-flow/core'
import '@vue-flow/core/dist/style.css'
import type { NodeInstance, Connection } from '../types/domain'

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

const flowNodes = computed<FlowNode[]>(() =>
  props.nodes.map((n) => ({
    id: n.id,
    position: { x: n.position[0], y: n.position[1] },
    label: `${n.id}\n${n.node_type}`,
    data: { nodeType: n.node_type, disabled: n.disabled },
  })),
)

const flowEdges = computed<FlowEdge[]>(() =>
  props.connections.map((c) => ({
    id: `${c.from_node}:${c.from_output}->${c.to_node}:${c.to_input}`,
    source: c.from_node,
    target: c.to_node,
    sourceHandle: String(c.from_output),
    targetHandle: String(c.to_input),
  })),
)

onNodeClick((event) => {
  emit('node-select', event.node.id)
})

onNodeDragStop((event) => {
  emit('node-move', event.node.id, [event.node.position.x, event.node.position.y])
})

onConnect((connection) => {
  emit('connect', {
    from_node: connection.source,
    from_output: Number(connection.sourceHandle ?? 0),
    to_node: connection.target,
    to_input: Number(connection.targetHandle ?? 0),
  })
})
</script>

<template>
  <div class="w-full h-full">
    <VueFlow :nodes="flowNodes" :edges="flowEdges" fit-view-on-init>
      <template #node-default="{ data, label }">
        <div class="px-3 py-2 rounded border bg-white shadow text-xs whitespace-pre-line" :class="{ 'opacity-50': data.disabled }">
          {{ label }}
        </div>
      </template>
    </VueFlow>
  </div>
</template>
