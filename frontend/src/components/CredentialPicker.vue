<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useCredentialsStore } from '../stores/credentials'
import { useNodeTypesStore } from '../stores/nodeTypes'
import type { CredentialSummary } from '../types/domain'

const props = defineProps<{ modelValue: string | null; nodeType: string }>()
const emit = defineEmits<{ 'update:modelValue': [id: string | null] }>()

const store = useCredentialsStore()
const nodeTypesStore = useNodeTypesStore()
const creating = ref(false)
const newName = ref('')
const newType = ref('')
const newDataText = ref('{}')
const error = ref('')

if (!store.loaded) {
  store.fetchAll().catch(() => {
    error.value = 'Failed to load credentials.'
  })
}
if (!nodeTypesStore.loaded) {
  nodeTypesStore.fetchAll().catch(() => {})
}

const acceptedTypes = computed(
  () => nodeTypesStore.types.find((t) => t.type_name === props.nodeType)?.credential_types ?? [],
)

watch(
  () => props.nodeType,
  () => {
    creating.value = false
    newType.value = ''
  },
)

function toggleCreating() {
  creating.value = !creating.value
  if (creating.value && acceptedTypes.value.length === 1) {
    newType.value = acceptedTypes.value[0]
  }
}

async function createCredential() {
  error.value = ''
  let data: Record<string, unknown>
  try {
    data = JSON.parse(newDataText.value)
  } catch {
    error.value = 'Data must be valid JSON.'
    return
  }
  let summary: CredentialSummary
  try {
    summary = await store.create(newName.value, newType.value, data)
  } catch {
    // Keep the form and its contents so the user can correct and retry.
    error.value = 'Failed to create credential.'
    return
  }
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
    <button type="button" class="text-xs text-blue-600" @click="toggleCreating">+ New credential</button>
    <div v-if="creating" class="border rounded p-2 space-y-2 bg-gray-50">
      <input v-model="newName" placeholder="Name" class="w-full border rounded px-2 py-1 text-sm" />
      <select v-if="acceptedTypes.length > 0" v-model="newType" class="w-full border rounded px-2 py-1 text-sm">
        <option value="" disabled>Select a type…</option>
        <option v-for="t in acceptedTypes" :key="t" :value="t">{{ t }}</option>
      </select>
      <input v-else v-model="newType" placeholder="Type (e.g. telegramApi)" class="w-full border rounded px-2 py-1 text-sm" />
      <textarea v-model="newDataText" rows="3" placeholder="{}" class="w-full border rounded px-2 py-1 text-xs font-mono"></textarea>
      <button type="button" class="text-xs bg-blue-600 text-white rounded px-2 py-1" @click="createCredential">Create</button>
    </div>
    <!-- Outside the create form so load failures are visible too. -->
    <p v-if="error" class="text-xs text-red-600">{{ error }}</p>
  </div>
</template>
