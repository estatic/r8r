<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useCredentialsStore } from '../stores/credentials'
import { useNodeTypesStore } from '../stores/nodeTypes'
import { useCredentialTypesStore } from '../stores/credentialTypes'
import type { CredentialSummary } from '../types/domain'

const props = defineProps<{ modelValue: string | null; nodeType: string }>()
const emit = defineEmits<{ 'update:modelValue': [id: string | null] }>()

const store = useCredentialsStore()
const nodeTypesStore = useNodeTypesStore()
const credentialTypesStore = useCredentialTypesStore()
const creating = ref(false)
const newName = ref('')
const newType = ref('')
const useCustomType = ref(false)
const fieldValues = ref<Record<string, string>>({})
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
if (!credentialTypesStore.loaded) {
  credentialTypesStore.fetchAll().catch(() => {})
}

const acceptedTypes = computed(
  () => nodeTypesStore.types.find((t) => t.type_name === props.nodeType)?.credential_types ?? [],
)
const hasRestrictedTypes = computed(() => acceptedTypes.value.length > 0)
const genericTypes = computed(() => credentialTypesStore.types.filter((t) => t.generic))
const schemaForSelectedType = computed(
  () => credentialTypesStore.types.find((t) => t.credential_type === newType.value) ?? null,
)

watch(
  () => props.nodeType,
  () => {
    creating.value = false
    newType.value = ''
    useCustomType.value = false
    fieldValues.value = {}
  },
)

function toggleCreating() {
  creating.value = !creating.value
  fieldValues.value = {}
  useCustomType.value = false
  newType.value = creating.value && acceptedTypes.value.length === 1 ? acceptedTypes.value[0] : ''
}

// acceptedTypes may still be empty at the moment `toggleCreating` runs (the
// node-types fetch can resolve after the click), so re-apply the
// single-accepted-type auto-select once the real list arrives.
watch(acceptedTypes, (types) => {
  if (creating.value && types.length === 1) {
    newType.value = types[0]
  }
})

function selectGenericType(value: string) {
  if (value === '__custom__') {
    useCustomType.value = true
    newType.value = ''
  } else {
    useCustomType.value = false
    newType.value = value
  }
  fieldValues.value = {}
}

async function createCredential() {
  error.value = ''
  let data: Record<string, unknown>
  const schema = schemaForSelectedType.value
  if (schema) {
    const missing = schema.fields.filter((f) => f.required && !fieldValues.value[f.name]?.trim())
    if (missing.length > 0) {
      error.value = `${missing.map((f) => f.label).join(', ')} ${missing.length === 1 ? 'is' : 'are'} required.`
      return
    }
    data = { ...fieldValues.value }
  } else {
    try {
      data = JSON.parse(newDataText.value)
    } catch {
      error.value = 'Data must be valid JSON.'
      return
    }
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
  useCustomType.value = false
  fieldValues.value = {}
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

      <select v-if="hasRestrictedTypes" v-model="newType" class="w-full border rounded px-2 py-1 text-sm">
        <option value="" disabled>Select a type…</option>
        <option v-for="t in acceptedTypes" :key="t" :value="t">{{ t }}</option>
      </select>
      <select
        v-else-if="!useCustomType"
        :value="newType"
        class="w-full border rounded px-2 py-1 text-sm"
        @change="selectGenericType(($event.target as HTMLSelectElement).value)"
      >
        <option value="" disabled>Select a type…</option>
        <option v-for="t in genericTypes" :key="t.credential_type" :value="t.credential_type">{{ t.display_name }}</option>
        <option value="__custom__">Custom…</option>
      </select>
      <input v-else v-model="newType" placeholder="Type (e.g. telegramApi)" class="w-full border rounded px-2 py-1 text-sm" />

      <div v-if="schemaForSelectedType" class="space-y-1">
        <input
          v-for="f in schemaForSelectedType.fields"
          :key="f.name"
          v-model="fieldValues[f.name]"
          :type="f.field_type === 'password' ? 'password' : 'text'"
          :placeholder="f.label"
          class="w-full border rounded px-2 py-1 text-sm"
        />
      </div>
      <textarea v-else v-model="newDataText" rows="3" placeholder="{}" class="w-full border rounded px-2 py-1 text-xs font-mono"></textarea>

      <button type="button" class="text-xs bg-blue-600 text-white rounded px-2 py-1" @click="createCredential">Create</button>
    </div>
    <!-- Outside the create form so load failures are visible too. -->
    <p v-if="error" class="text-xs text-red-600">{{ error }}</p>
  </div>
</template>
