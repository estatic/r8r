<script setup lang="ts">
import type { Execution, Item } from '../types/domain'

const props = defineProps<{ execution: Execution | null; history?: Execution[] }>()
const emit = defineEmits<{ close: []; select: [execution: Execution] }>()

/** The error of a node that failed in this run, else null. */
function errorOf(nodeId: string, items: Item[]): string | null {
  if (props.execution?.node_runs?.[nodeId]?.status !== 'error') return null
  const error = (items[0]?.json as Record<string, unknown> | undefined)?.error
  return typeof error === 'string' ? error : null
}

function onHistorySelect(e: Event) {
  const id = (e.target as HTMLSelectElement).value
  const chosen = (props.history ?? []).find((run) => run.id === id)
  if (chosen) emit('select', chosen)
}
</script>

<template>
  <aside v-if="execution" class="absolute bottom-0 left-0 right-0 h-64 bg-white border-t shadow-lg flex flex-col">
    <header class="px-4 py-2 border-b flex justify-between items-center gap-3">
      <span
        class="text-sm font-medium"
        :class="{
          'text-green-600': execution.status === 'Success',
          'text-red-600': execution.status === 'Error',
          'text-gray-500': execution.status === 'Running',
          'text-amber-600': execution.status === 'Canceled',
        }"
      >
        {{ execution.status }}
      </span>
      <select
        v-if="history && history.length > 0"
        :value="execution.id"
        class="text-xs border rounded px-1 py-0.5"
        @change="onHistorySelect"
      >
        <option v-for="run in history" :key="run.id" :value="run.id">{{ run.started_at }} — {{ run.status }}</option>
      </select>
      <div class="flex-1"></div>
      <button class="text-gray-400" @click="$emit('close')">&times;</button>
    </header>
    <div class="p-4 overflow-auto flex-1 space-y-3">
      <div v-for="(items, nodeId) in execution.node_outputs" :key="nodeId">
        <div class="text-xs font-medium text-gray-500 mb-1">{{ nodeId }}</div>
        <!-- A failed node: its error (with any stack trace) as text, line by line. -->
        <pre
          v-if="errorOf(String(nodeId), items) !== null"
          data-testid="node-error"
          class="bg-red-50 text-red-800 border border-red-200 rounded p-2 text-xs overflow-auto whitespace-pre-wrap"
          >{{ errorOf(String(nodeId), items) }}</pre
        >
        <pre v-else class="bg-gray-50 rounded p-2 text-xs overflow-auto">{{ JSON.stringify(items, null, 2) }}</pre>
      </div>
      <p v-if="Object.keys(execution.node_outputs).length === 0" class="text-gray-400 text-sm">No node output.</p>
    </div>
  </aside>
</template>
