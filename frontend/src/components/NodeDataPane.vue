<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { cellText, tableColumns, type PortData } from '../canvas/nodeData'

/** One side of the node view: the items in or out (a tab per output), or the node's error. */
const props = defineProps<{ title: string; ports: PortData[] | null; error?: string | null; emptyText: string; reused?: boolean }>()

const SHOWN = 100
const tab = ref(0)
const view = ref<'table' | 'json'>('table')
watch(
  () => props.ports?.length,
  () => {
    // Open on the first output that sent something.
    const first = props.ports?.findIndex((p) => p.items.length > 0) ?? -1
    tab.value = Math.max(0, first)
  },
  { immediate: true },
)
const current = computed(() => props.ports?.[tab.value] ?? null)
const items = computed(() => current.value?.items ?? [])
const columns = computed(() => tableColumns(items.value))
const plural = (n: number) => (n === 1 ? '1 item' : `${n} items`)
</script>

<template>
  <section class="flex flex-col min-w-0 min-h-0 bg-white" :data-testid="`pane-${title.toLowerCase()}`">
    <header class="px-3 py-2 border-b flex items-center gap-2 flex-wrap">
      <span class="text-xs font-semibold uppercase tracking-wide text-gray-500">{{ title }}</span>
      <span v-if="reused" class="text-[10px] text-green-700 bg-green-50 border border-green-200 rounded px-1">reused from the last run</span>
      <template v-if="!error && ports && ports.length > 0">
        <div v-if="ports.length > 1" class="flex gap-1">
          <button
            v-for="(p, i) in ports"
            :key="p.label"
            type="button"
            class="text-xs rounded px-2 py-0.5 border"
            :class="i === tab ? (p.label === 'error' ? 'bg-red-600 text-white border-red-600' : 'bg-gray-800 text-white border-gray-800') : 'bg-white text-gray-600'"
            :data-testid="`port-${p.label}`"
            @click="tab = i"
          >
            {{ p.label }} ({{ p.items.length }})
          </button>
        </div>
        <span v-else class="text-xs text-gray-500">{{ plural(items.length) }}</span>
        <div class="flex-1"></div>
        <div class="flex text-xs border rounded overflow-hidden">
          <button type="button" class="px-2" :class="view === 'table' ? 'bg-gray-200' : 'bg-white'" data-testid="view-table" @click="view = 'table'">Table</button>
          <button type="button" class="px-2" :class="view === 'json' ? 'bg-gray-200' : 'bg-white'" data-testid="view-json" @click="view = 'json'">JSON</button>
        </div>
      </template>
    </header>
    <div class="flex-1 overflow-auto p-3 min-h-0">
      <div v-if="error" data-testid="node-error-details">
        <p class="text-sm font-medium text-red-700 mb-1">The node failed</p>
        <pre class="bg-red-50 text-red-800 border border-red-200 rounded p-2 text-xs whitespace-pre-wrap break-words">{{ error }}</pre>
      </div>
      <p v-else-if="!ports" class="text-sm text-gray-400">{{ emptyText }}</p>
      <p v-else-if="items.length === 0" class="text-sm text-gray-400">No items{{ ports.length > 1 ? ` on ${current?.label}` : '' }}.</p>
      <template v-else-if="view === 'json'">
        <pre class="text-xs bg-gray-50 rounded p-2 whitespace-pre-wrap break-words" data-testid="items-json">{{ JSON.stringify(items.slice(0, SHOWN).map((i) => i.json), null, 2) }}</pre>
      </template>
      <table v-else-if="columns.length > 0" class="text-xs border-collapse w-full" data-testid="items-table">
        <thead>
          <tr>
            <th class="border px-1.5 py-1 bg-gray-50 text-gray-500 font-normal text-left">#</th>
            <th v-for="c in columns" :key="c" class="border px-1.5 py-1 bg-gray-50 text-left font-medium">{{ c }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="(it, r) in items.slice(0, SHOWN)" :key="r">
            <td class="border px-1.5 py-1 text-gray-400">{{ r }}</td>
            <td v-for="c in columns" :key="c" class="border px-1.5 py-1 align-top font-mono break-all">{{ cellText((it.json as Record<string, unknown>)?.[c]) }}</td>
          </tr>
        </tbody>
      </table>
      <pre v-else class="text-xs bg-gray-50 rounded p-2">{{ JSON.stringify(items.slice(0, SHOWN).map((i) => i.json), null, 2) }}</pre>
      <p v-if="items.length > SHOWN" class="text-xs text-gray-400 mt-1">Showing the first {{ SHOWN }} of {{ items.length }}.</p>
    </div>
  </section>
</template>
