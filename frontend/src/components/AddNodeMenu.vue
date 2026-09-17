<script setup lang="ts">
import { computed, ref } from 'vue'
import { useNodeTypesStore } from '../stores/nodeTypes'

const emit = defineEmits<{ add: [nodeType: string] }>()
const store = useNodeTypesStore()
const open = ref(false)
const search = ref('')

store.fetchAll()

const filtered = computed(() => store.types.filter((t) => t.toLowerCase().includes(search.value.toLowerCase())))

function choose(type: string) {
  emit('add', type)
  open.value = false
  search.value = ''
}
</script>

<template>
  <div class="relative">
    <button class="bg-blue-600 text-white rounded px-3 py-1.5 text-sm" @click="open = !open">+ Add node</button>
    <div v-if="open" class="absolute z-10 mt-1 w-64 bg-white border rounded shadow">
      <input v-model="search" autofocus placeholder="Search node types…" class="w-full border-b px-3 py-2 text-sm" />
      <ul class="max-h-64 overflow-auto">
        <li v-for="t in filtered" :key="t" class="px-3 py-2 text-sm hover:bg-gray-50 cursor-pointer" @click="choose(t)">
          {{ t }}
        </li>
        <li v-if="filtered.length === 0" class="px-3 py-2 text-sm text-gray-400">No matches</li>
      </ul>
    </div>
  </div>
</template>
