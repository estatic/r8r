<script setup lang="ts">
import { computed } from 'vue'
import { BaseEdge, EdgeLabelRenderer, getBezierPath, type EdgeProps } from '@vue-flow/core'

/** A link that can carry an "N items" label, drawn just above the line. */
const props = defineProps<
  EdgeProps<{ itemsLabel?: string; color?: string; hovered?: boolean; onInsert?: () => void; onDelete?: () => void; onHover?: (on: boolean) => void }>
>()

const path = computed(() =>
  getBezierPath({
    sourceX: props.sourceX,
    sourceY: props.sourceY,
    sourcePosition: props.sourcePosition,
    targetX: props.targetX,
    targetY: props.targetY,
    targetPosition: props.targetPosition,
  }),
)
</script>

<template>
  <BaseEdge :id="id" :path="path[0]" :marker-end="markerEnd" :style="style" />
  <EdgeLabelRenderer v-if="data?.itemsLabel">
    <div
      data-testid="edge-items"
      class="absolute text-[10px] font-medium px-1 rounded bg-white/90 pointer-events-none"
      :style="{
        transform: `translate(-50%, -100%) translate(${path[1]}px, ${path[2] - 4}px)`,
        color: data.color ?? '#4b5563',
      }"
    >
      {{ data.itemsLabel }}
    </div>
  </EdgeLabelRenderer>
  <EdgeLabelRenderer>
    <!-- Shown while the link is hovered: put a node into it, or delete it. -->
    <div
      v-show="data?.hovered"
      class="absolute flex gap-1 nodrag nopan"
      :style="{ transform: `translate(-50%, 4px) translate(${path[1]}px, ${path[2]}px)`, pointerEvents: 'all' }"
      @mouseenter="data?.onHover?.(true)"
      @mouseleave="data?.onHover?.(false)"
    >
      <button
        data-testid="edge-insert"
        title="Add a node here"
        class="w-5 h-5 leading-none rounded border border-gray-400 bg-white text-gray-700 text-sm hover:bg-blue-50"
        @click.stop="data?.onInsert?.()"
      >
        +
      </button>
      <button
        data-testid="edge-delete"
        title="Delete this connection"
        class="w-5 h-5 leading-none rounded border border-gray-400 bg-white text-xs hover:bg-red-50"
        @click.stop="data?.onDelete?.()"
      >
        🗑
      </button>
    </div>
  </EdgeLabelRenderer>
</template>
