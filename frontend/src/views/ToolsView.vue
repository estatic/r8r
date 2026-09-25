<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { inUseToolWorkflowNames, useToolsStore } from '../stores/tools'
import { TOOL_NODE_TYPES } from '../tools/schema'
import ToolForm from '../components/ToolForm.vue'
import type { Tool } from '../types/domain'

const store = useToolsStore()
const creating = ref(false)
const editingId = ref<string | null>(null)
const error = ref('')

onMounted(() => {
  store.fetchAll().catch(() => {
    error.value = 'Failed to load tools.'
  })
})

function nodeTypeLabel(type: string): string {
  return TOOL_NODE_TYPES.find((t) => t.value === type)?.label ?? type
}

function usage(t: Tool): string {
  if (!t.used_by) return 'not used'
  return `used by ${t.used_by} workflow${t.used_by === 1 ? '' : 's'}`
}

async function remove(t: Tool) {
  error.value = ''
  if (!window.confirm(`Delete tool "${t.name}"?`)) return
  try {
    await store.remove(t.id)
  } catch (e) {
    const names = inUseToolWorkflowNames(e)
    error.value = names
      ? `Can't delete "${t.name}": used by ${names.join(', ')}. Remove it from those agents first.`
      : 'Failed to delete tool.'
  }
}

function afterSave() {
  creating.value = false
  editingId.value = null
}
</script>

<template>
  <main class="min-h-screen bg-gray-50">
    <header class="bg-white border-b px-6 py-4 flex justify-between items-center">
      <h1 class="text-xl font-semibold text-gray-800">Tools</h1>
      <div class="flex gap-4">
        <router-link to="/workflows" class="text-sm text-blue-600">Workflows</router-link>
        <router-link to="/credentials" class="text-sm text-blue-600">Credentials</router-link>
      </div>
    </header>
    <div class="p-6 max-w-3xl mx-auto space-y-4">
      <p class="text-sm text-gray-600">Tools are actions an AI Agent can decide to call. Define one here, then tick it in an agent's settings.</p>
      <button type="button" data-testid="new-tool" class="bg-blue-600 text-white rounded px-4 py-2 text-sm" @click="creating = !creating; editingId = null">
        + New tool
      </button>
      <ToolForm v-if="creating" @saved="afterSave" @cancel="creating = false" />
      <p v-if="error" role="alert" class="text-sm text-red-600">{{ error }}</p>
      <ul class="divide-y bg-white rounded shadow">
        <li v-for="t in store.tools" :key="t.id" data-testid="tool-row" class="px-4 py-3 space-y-2">
          <div class="flex justify-between items-center gap-4">
            <div>
              <div class="font-medium text-gray-800 font-mono">{{ t.name }}</div>
              <div class="text-xs text-gray-500">{{ nodeTypeLabel(t.node_type) }} · {{ usage(t) }}</div>
              <div class="text-xs text-gray-600">{{ t.description }}</div>
            </div>
            <div class="flex gap-3 text-sm">
              <button type="button" class="text-blue-600" :aria-label="`Edit ${t.name}`" @click="editingId = editingId === t.id ? null : t.id; creating = false">Edit</button>
              <button type="button" data-testid="delete-tool" class="text-red-600" :aria-label="`Delete ${t.name}`" @click="remove(t)">Delete</button>
            </div>
          </div>
          <ToolForm v-if="editingId === t.id" :key="t.id" :tool-id="t.id" @saved="afterSave" @cancel="editingId = null" />
        </li>
      </ul>
      <p v-if="store.loaded && store.tools.length === 0" class="text-sm text-gray-500">No tools yet.</p>
    </div>
  </main>
</template>
