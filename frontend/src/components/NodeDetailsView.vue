<script setup lang="ts">
import { computed } from 'vue'
import NodeDataPane from './NodeDataPane.vue'
import { nodeError, nodeInput, nodeOutputs } from '../canvas/nodeData'
import { useNodeTypesStore } from '../stores/nodeTypes'
import type { Execution, NodeInstance, Workflow } from '../types/domain'

/**
 * The node view, as n8n's: what came in on the left, the node's settings in
 * the middle (the slot), what it sent on the right -- or why it failed.
 */
const props = defineProps<{ node: NodeInstance; workflow: Workflow; execution: Execution | null }>()
defineEmits<{ close: [] }>()

const nodeTypes = useNodeTypesStore()
const portNames = computed(() => nodeTypes.types.find((t) => t.type_name === props.node.node_type)?.output_ports ?? [])
const run = computed(() => props.execution?.node_runs?.[props.node.id])
const hasInputs = computed(() => props.workflow.connections.some((c) => c.to_node === props.node.id))
const input = computed(() => {
  const items = nodeInput(props.execution, props.workflow, props.node.id)
  return items === null ? null : [{ label: 'Input', items }]
})
const outputs = computed(() => nodeOutputs(props.execution, props.node.id, portNames.value))
const error = computed(() => nodeError(props.execution, props.node.id))
const noRunText = computed(() =>
  props.execution ? 'This node didn\'t run in the run on screen.' : 'Execute the workflow to see the data here.',
)
</script>

<template>
  <div class="absolute inset-0 z-30 bg-black/30 flex p-4" data-testid="node-details" @click.self="$emit('close')">
    <div class="flex-1 flex min-w-0 rounded-lg overflow-hidden shadow-2xl border bg-white">
      <NodeDataPane
        class="flex-1 border-r"
        title="Input"
        :ports="hasInputs ? input : null"
        :empty-text="hasInputs ? noRunText : 'A trigger starts the workflow: it has no input.'"
      />
      <slot />
      <NodeDataPane class="flex-1 border-l" title="Output" :ports="outputs" :error="error" :empty-text="noRunText" :reused="run?.reused" />
    </div>
  </div>
</template>
