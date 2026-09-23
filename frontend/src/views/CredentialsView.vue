<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { inUseWorkflowNames, useCredentialsStore } from '../stores/credentials'
import CredentialForm from '../components/CredentialForm.vue'
import type { CredentialSummary } from '../types/domain'

const store = useCredentialsStore()
const creating = ref(false)
const editingId = ref<string | null>(null)
const error = ref('')

onMounted(() => {
  store.fetchAll().catch(() => {
    error.value = 'Failed to load credentials.'
  })
})

function usage(c: CredentialSummary): string {
  if (!c.used_by) return 'not used'
  return `used by ${c.used_by} workflow${c.used_by === 1 ? '' : 's'}`
}

async function remove(c: CredentialSummary) {
  error.value = ''
  if (!window.confirm(`Delete credential "${c.name}"?`)) return
  try {
    await store.remove(c.id)
  } catch (e) {
    const names = inUseWorkflowNames(e)
    error.value = names
      ? `Can't delete "${c.name}": used by ${names.join(', ')}. Remove it from those workflows first.`
      : 'Failed to delete credential.'
  }
}

async function afterSave() {
  creating.value = false
  editingId.value = null
  await store.fetchAll().catch(() => {})
}
</script>

<template>
  <main class="min-h-screen bg-gray-50">
    <header class="bg-white border-b px-6 py-4 flex justify-between items-center">
      <h1 class="text-xl font-semibold text-gray-800">Credentials</h1>
      <router-link to="/workflows" class="text-sm text-blue-600">Workflows</router-link>
    </header>
    <div class="p-6 max-w-3xl mx-auto space-y-4">
      <button type="button" class="bg-blue-600 text-white rounded px-4 py-2 text-sm" @click="creating = !creating; editingId = null">
        + New credential
      </button>
      <CredentialForm v-if="creating" mode="create" offer-all-types @saved="afterSave" @cancel="creating = false" />
      <p v-if="error" class="text-sm text-red-600">{{ error }}</p>
      <ul class="divide-y bg-white rounded shadow">
        <li v-for="c in store.credentials" :key="c.id" data-testid="credential-row" class="px-4 py-3 space-y-2">
          <div class="flex justify-between items-center gap-4">
            <div>
              <div class="font-medium text-gray-800">{{ c.name }}</div>
              <div class="text-xs text-gray-500">
                <span class="font-mono">{{ c.credential_type }}</span> · {{ usage(c) }} · updated {{ new Date(c.updated_at).toLocaleString() }}
              </div>
            </div>
            <div class="flex gap-3 text-sm">
              <button type="button" class="text-blue-600" @click="editingId = editingId === c.id ? null : c.id; creating = false">Edit</button>
              <button type="button" data-testid="delete-credential" class="text-red-600" @click="remove(c)">Delete</button>
            </div>
          </div>
          <CredentialForm v-if="editingId === c.id" :key="c.id" mode="edit" :credential-id="c.id" @saved="afterSave" @cancel="editingId = null" />
        </li>
      </ul>
      <p v-if="store.loaded && store.credentials.length === 0" class="text-sm text-gray-500">No credentials yet.</p>
    </div>
  </main>
</template>