<script setup lang="ts">
import { computed, ref } from 'vue'
import { useNodeTypesStore } from '../stores/nodeTypes'

const emit = defineEmits<{ add: [nodeType: string] }>()
const store = useNodeTypesStore()
const open = ref(false)
const search = ref('')
const error = ref('')

store.fetchAll().catch(() => {
  error.value = 'Failed to load node types.'
})

const filtered = computed(() => {
  const q = search.value.toLowerCase()
  return store.types.filter((t) => t.display_name.toLowerCase().includes(q) || t.type_name.toLowerCase().includes(q))
})

function choose(type: string) {
  emit('add', type)
  open.value = false
  search.value = ''
}
</script>

<template>
  <div class="relative">
    <button class="bg-blue-600 text-white rounded px-3 py-1.5 text-sm" @click="open = !open">+ Add node</button>
    <div v-if="open" class="absolute z-10 mt-1 w-72 bg-white border rounded shadow">
      <input v-model="search" autofocus placeholder="Search node types…" class="w-full border-b px-3 py-2 text-sm" />
      <p v-if="error" class="px-3 py-2 text-xs text-red-600">{{ error }}</p>
      <ul class="max-h-64 overflow-auto">
        <li
          v-for="t in filtered"
          :key="t.type_name"
          class="px-3 py-2 text-sm hover:bg-gray-50 cursor-pointer"
          @click="choose(t.type_name)"
        >
          <div>{{ t.icon }} {{ t.display_name }}</div>
          <div class="text-xs text-gray-400">{{ t.description }}</div>
        </li>
        <li v-if="filtered.length === 0" class="px-3 py-2 text-sm text-gray-400">No matches</li>
      </ul>
    </div>
  </div>
</template>
