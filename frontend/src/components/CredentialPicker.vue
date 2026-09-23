<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useCredentialsStore } from '../stores/credentials'
import { useNodeTypesStore } from '../stores/nodeTypes'
import CredentialForm from './CredentialForm.vue'
import type { CredentialSummary } from '../types/domain'

const props = defineProps<{ modelValue: string | null; nodeType: string }>()
const emit = defineEmits<{ 'update:modelValue': [id: string | null] }>()

const store = useCredentialsStore()
const nodeTypesStore = useNodeTypesStore()
const creating = ref(false)
const editing = ref(false)
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
    editing.value = false
  },
)

function toggleCreating() {
  creating.value = !creating.value
  editing.value = false
}

function onCreated(summary: CredentialSummary) {
  emit('update:modelValue', summary.id)
  creating.value = false
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
    <div class="flex gap-3">
      <button type="button" class="text-xs text-blue-600" @click="toggleCreating">+ New credential</button>
      <button
        v-if="modelValue"
        type="button"
        data-testid="edit-credential"
        class="text-xs text-blue-600"
        @click="editing = !editing; creating = false"
      >
        Edit
      </button>
    </div>
    <CredentialForm v-if="creating" :key="nodeType" mode="create" :accepted-types="acceptedTypes" @saved="onCreated" @cancel="creating = false" />
    <CredentialForm v-if="editing && modelValue" :key="modelValue" mode="edit" :credential-id="modelValue" @saved="editing = false" @cancel="editing = false" />
    <!-- Outside the forms so load failures are visible too. -->
    <p v-if="error" class="text-xs text-red-600">{{ error }}</p>
  </div>
</template>