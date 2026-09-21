<script setup lang="ts">
import { computed } from 'vue'
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

const flowNodes = computed<FlowNode[]>(() =>
  props.nodes.map((n) => ({
    id: n.id,
    position: { x: n.position[0], y: n.position[1] },
    label: labelFor(n.node_type),
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
    markerEnd: MarkerType.ArrowClosed,
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
    <VueFlow :nodes="flowNodes" :edges="flowEdges" fit-view-on-init :delete-key-code="null">
      <!--
        A #node-default slot *replaces* Vue Flow's DefaultNode component
        entirely (NodeWrapper prefers the slot over the registered node type),
        and DefaultNode is what normally renders the target/source <Handle>
        elements. Without them there is no `.vue-flow__handle` element for a
        user to drag a connection from, so we render them ourselves. Handle id
        "0" matches the `sourceHandle`/`targetHandle` values derived from
        `from_output`/`to_input` in `flowEdges` (always 0 in r8r's
        single-input/single-output node model), so loaded connections attach to
        these handles rather than falling back to node bounds.
      -->
      <template #node-default="{ data, label }">
        <!-- Vue Flow's optional theme-default.css isn't imported, so give the
             handles their own visible size/colour here. -->
        <Handle id="0" type="target" :position="Position.Top" class="w-2.5 h-2.5 rounded-full bg-gray-500 border border-white" />
        <div
          class="px-3 py-2 rounded border bg-white shadow text-xs whitespace-pre-line"
          :class="{ 'opacity-50': data.disabled }"
          :title="data.nodeType"
        >
          {{ label }}
        </div>
        <Handle id="0" type="source" :position="Position.Bottom" class="w-2.5 h-2.5 rounded-full bg-gray-500 border border-white" />
      </template>
    </VueFlow>
  </div>
</template>
