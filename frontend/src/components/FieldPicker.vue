<script setup lang="ts">
import { computed, ref } from 'vue'
import type { UpstreamSource } from '../canvas/inputData'

/** A searchable list of the fields reaching a node; emits the one picked. */
const props = defineProps<{ sources: UpstreamSource[]; nodeLabels: Record<string, string> }>()
const emit = defineEmits<{ pick: [expression: string, path: string] }>()

const search = ref('')
const hasData = computed(() => props.sources.some((s) => s.fields.length > 0))
const filteredSources = computed(() => {
  const q = search.value.toLowerCase()
  return props.sources
    .map((s) => ({ ...s, fields: s.fields.filter((f) => !q || f.path.toLowerCase().includes(q) || f.preview.toLowerCase().includes(q)) }))
    .filter((s) => s.fields.length > 0)
})
</script>

<template>
  <div class="border rounded bg-white shadow-sm" data-testid="field-picker">
    <p v-if="!hasData" class="px-2 py-1.5 text-xs text-gray-500">
      {{
        sources.length === 0
          ? 'Nothing is connected to this node\'s input yet.'
          : 'Run the workflow once (or send a test message) to pick from the data reaching this node.'
      }}
    </p>
    <template v-else>
      <input v-model="search" placeholder="Search fields…" class="w-full border-b px-2 py-1 text-xs" />
      <div class="max-h-56 overflow-auto">
        <div v-for="s in filteredSources" :key="s.nodeId">
          <div class="px-2 pt-1.5 pb-0.5 text-[10px] uppercase tracking-wide text-gray-400">
            {{ nodeLabels[s.nodeId] ?? s.nodeId }}{{ s.direct ? ' — input' : '' }}
          </div>
          <button
            v-for="f in s.fields"
            :key="f.path"
            type="button"
            class="w-full text-left px-2 py-0.5 hover:bg-blue-50 flex justify-between gap-2"
            :title="f.expression"
            @click="emit('pick', f.expression, f.path)"
          >
            <span class="font-mono text-xs text-gray-800 truncate">{{ f.path.replace(/^\./, '') }}</span>
            <span class="text-xs text-gray-400 truncate max-w-[45%]">{{ f.preview }}</span>
          </button>
        </div>
        <p v-if="filteredSources.length === 0" class="px-2 py-1.5 text-xs text-gray-400">No matches</p>
      </div>
    </template>
  </div>
</template>
