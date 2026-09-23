<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useCredentialsStore } from '../stores/credentials'
import { useCredentialTypesStore } from '../stores/credentialTypes'

const props = defineProps<{
  mode: 'create' | 'edit'
  acceptedTypes?: string[]
  credentialId?: string
  offerAllTypes?: boolean
}>()
const emit = defineEmits<{ saved: [summary: import('../types/domain').CredentialSummary]; cancel: [] }>()

const store = useCredentialsStore()
const credentialTypesStore = useCredentialTypesStore()
const newName = ref('')
const newType = ref('')
const useCustomType = ref(false)
const fieldValues = ref<Record<string, string>>({})
const newDataText = ref(props.mode === 'create' ? '{}' : '')
const error = ref('')
const editing = props.mode === 'edit'

if (!credentialTypesStore.loaded) {
  credentialTypesStore.fetchAll().catch(() => {})
}

const accepted = computed(() => props.acceptedTypes ?? [])
const hasRestrictedTypes = computed(() => !editing && accepted.value.length > 0)
const selectableTypes = computed(() =>
  props.offerAllTypes ? credentialTypesStore.types : credentialTypesStore.types.filter((t) => t.generic),
)
const schemaForSelectedType = computed(
  () => credentialTypesStore.types.find((t) => t.credential_type === newType.value) ?? null,
)

// Create mode: a type change drops values typed for the previous schema so
// they can't leak into the submission. Edit mode sets the type once while
// pre-filling, so it must not clear.
watch(newType, () => {
  if (!editing) fieldValues.value = {}
  error.value = ''
})

// Create mode: auto-select a single accepted type, and drop a type picked
// from the generic list before the node's restricted list arrived.
watch(
  accepted,
  (types) => {
    if (editing) return
    if (types.length === 1) {
      newType.value = types[0]
    } else if (types.length > 0 && !types.includes(newType.value)) {
      newType.value = ''
      useCustomType.value = false
    }
  },
  { immediate: true },
)

if (editing && props.credentialId) {
  store
    .get(props.credentialId)
    .then((detail) => {
      newName.value = detail.name
      newType.value = detail.credential_type
      fieldValues.value = { ...detail.fields }
    })
    .catch(() => {
      error.value = 'Failed to load credential.'
    })
}

function selectGenericType(value: string) {
  if (value === '__custom__') {
    useCustomType.value = true
    newType.value = ''
  } else {
    useCustomType.value = false
    newType.value = value
  }
}

function trimmedFieldData(schemaFields: { name: string }[]): Record<string, unknown> {
  // Built from the schema (not the accumulated map) and trimmed; blanks
  // dropped -- omitted on create, "keep current" on edit.
  return Object.fromEntries(
    schemaFields.map((f) => [f.name, (fieldValues.value[f.name] ?? '').trim()]).filter(([, v]) => v !== ''),
  )
}

async function submit() {
  error.value = ''
  const schema = schemaForSelectedType.value
  if (editing) {
    if (!newName.value.trim()) {
      error.value = 'Name is required.'
      return
    }
    const patch: { name: string; data?: Record<string, unknown> } = { name: newName.value.trim() }
    if (schema) {
      const data = trimmedFieldData(schema.fields)
      if (Object.keys(data).length > 0) patch.data = data
    } else if (newDataText.value.trim()) {
      try {
        patch.data = JSON.parse(newDataText.value)
      } catch {
        error.value = 'Data must be valid JSON.'
        return
      }
    }
    try {
      emit('saved', await store.update(props.credentialId!, patch))
    } catch {
      error.value = 'Failed to save credential.'
    }
    return
  }

  let data: Record<string, unknown>
  if (schema) {
    const missing = schema.fields.filter((f) => f.required && !fieldValues.value[f.name]?.trim())
    if (missing.length > 0) {
      error.value = `${missing.map((f) => f.label).join(', ')} ${missing.length === 1 ? 'is' : 'are'} required.`
      return
    }
    data = trimmedFieldData(schema.fields)
  } else {
    try {
      data = JSON.parse(newDataText.value)
    } catch {
      error.value = 'Data must be valid JSON.'
      return
    }
  }
  try {
    emit('saved', await store.create(newName.value, newType.value, data))
  } catch {
    // Keep the form and its contents so the user can correct and retry.
    error.value = 'Failed to create credential.'
  }
}
</script>

<template>
  <div class="border rounded p-2 space-y-2 bg-gray-50">
    <!-- autocomplete opt-outs keep password managers from treating this as
         a login form and filling the r8r login into credential secrets. -->
    <input v-model="newName" placeholder="Name" aria-label="Name" autocomplete="off" class="w-full border rounded px-2 py-1 text-sm" />

    <p v-if="editing" class="text-xs text-gray-600">Type: <span class="font-mono">{{ newType }}</span></p>
    <template v-else>
      <select v-if="hasRestrictedTypes" v-model="newType" class="w-full border rounded px-2 py-1 text-sm">
        <option value="" disabled>Select a type…</option>
        <option v-for="t in accepted" :key="t" :value="t">{{ t }}</option>
      </select>
      <select
        v-else-if="!useCustomType"
        :value="newType"
        class="w-full border rounded px-2 py-1 text-sm"
        @change="selectGenericType(($event.target as HTMLSelectElement).value)"
      >
        <option value="" disabled>Select a type…</option>
        <option v-for="t in selectableTypes" :key="t.credential_type" :value="t.credential_type">{{ t.display_name }}</option>
        <option value="__custom__">Custom…</option>
      </select>
      <input v-else v-model="newType" placeholder="Type (e.g. telegramApi)" class="w-full border rounded px-2 py-1 text-sm" />
    </template>

    <div v-if="schemaForSelectedType" class="space-y-1">
      <input
        v-for="f in schemaForSelectedType.fields"
        :key="f.name"
        v-model="fieldValues[f.name]"
        :type="f.field_type === 'password' ? 'password' : 'text'"
        :placeholder="editing && f.field_type === 'password' ? '•••••• (unchanged)' : f.label"
        :aria-label="f.label"
        :autocomplete="f.field_type === 'password' ? 'new-password' : 'off'"
        class="w-full border rounded px-2 py-1 text-sm"
      />
    </div>
    <template v-else>
      <p v-if="editing" class="text-xs text-gray-500">Enter the full JSON to replace the stored data, or leave empty to keep it.</p>
      <textarea v-model="newDataText" rows="3" :placeholder="editing ? '' : '{}'" class="w-full border rounded px-2 py-1 text-xs font-mono"></textarea>
    </template>

    <div class="flex gap-2">
      <button type="button" class="text-xs bg-blue-600 text-white rounded px-2 py-1" @click="submit">{{ editing ? 'Save' : 'Create' }}</button>
      <button type="button" class="text-xs text-gray-600" @click="emit('cancel')">Cancel</button>
    </div>
    <p v-if="error" class="text-xs text-red-600">{{ error }}</p>
  </div>
</template>