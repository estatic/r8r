<script setup lang="ts">
import { ref } from 'vue'
import { useCredentialsStore } from '../stores/credentials'

const props = defineProps<{ modelValue: string | null }>()
const emit = defineEmits<{ 'update:modelValue': [id: string | null] }>()

const store = useCredentialsStore()
const creating = ref(false)
const newName = ref('')
const newType = ref('')
const newDataText = ref('{}')
const error = ref('')

if (!store.loaded) store.fetchAll()

async function createCredential() {
  error.value = ''
  let data: Record<string, unknown>
  try {
    data = JSON.parse(newDataText.value)
  } catch {
    error.value = 'Data must be valid JSON.'
    return
  }
  const summary = await store.create(newName.value, newType.value, data)
  emit('update:modelValue', summary.id)
  creating.value = false
  newName.value = ''
  newType.value = ''
  newDataText.value = '{}'
}
</script>

<template>
  <div class="space-y-2">
    <select
      :value="modelValue ?? ''"
      class="w-full border rounded px-2 py-1.5 text-sm"
      @change="emit('update:modelValue', ($event.target as HTMLSelectElement).value || null)"
    >
      <option value="">No credential</option>
      <option v-for="c in store.credentials" :key="c.id" :value="c.id">{{ c.name }} ({{ c.credential_type }})</option>
    </select>
    <button type="button" class="text-xs text-blue-600" @click="creating = !creating">+ New credential</button>
    <div v-if="creating" class="border rounded p-2 space-y-2 bg-gray-50">
      <input v-model="newName" placeholder="Name" class="w-full border rounded px-2 py-1 text-sm" />
      <input v-model="newType" placeholder="Type (e.g. telegramApi)" class="w-full border rounded px-2 py-1 text-sm" />
      <textarea v-model="newDataText" rows="3" placeholder="{}" class="w-full border rounded px-2 py-1 text-xs font-mono"></textarea>
      <p v-if="error" class="text-xs text-red-600">{{ error }}</p>
      <button type="button" class="text-xs bg-blue-600 text-white rounded px-2 py-1" @click="createCredential">Create</button>
    </div>
  </div>
</template>
