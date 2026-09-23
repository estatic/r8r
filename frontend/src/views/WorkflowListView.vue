<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import { useWorkflowsStore } from '../stores/workflows'
import { useAuthStore } from '../stores/auth'

const store = useWorkflowsStore()
const auth = useAuthStore()
const router = useRouter()
const newName = ref('')
const error = ref('')

onMounted(async () => {
  try {
    await store.fetchAll()
  } catch {
    error.value = 'Failed to load workflows.'
  }
})

async function createWorkflow() {
  if (!newName.value.trim()) return
  error.value = ''
  try {
    const workflow = await store.create(newName.value.trim())
    newName.value = ''
    router.push({ name: 'workflow-editor', params: { id: workflow.id } })
  } catch {
    error.value = 'Failed to create workflow.'
  }
}

async function removeWorkflow(id: string) {
  error.value = ''
  try {
    await store.remove(id)
  } catch {
    error.value = 'Failed to delete workflow.'
  }
}

async function setActive(id: string, active: boolean) {
  error.value = ''
  try {
    await store.setActive(id, active)
  } catch {
    error.value = active ? 'Failed to activate workflow.' : 'Failed to deactivate workflow.'
  }
}

function logout() {
  auth.logout()
  router.push({ name: 'login' })
}
</script>

<template>
  <main class="min-h-screen bg-gray-50">
    <header class="bg-white border-b px-6 py-4 flex justify-between items-center">
      <h1 class="text-xl font-semibold text-gray-800">Workflows</h1>
      <div class="flex gap-4 items-center">
        <router-link to="/credentials" class="text-sm text-blue-600">Credentials</router-link>
        <button class="text-sm text-gray-500" @click="logout">Log out</button>
      </div>
    </header>
    <div class="p-6 max-w-3xl mx-auto space-y-4">
      <form class="flex gap-2" @submit.prevent="createWorkflow">
        <input v-model="newName" placeholder="New workflow name" class="flex-1 border rounded px-3 py-2" />
        <button type="submit" class="bg-blue-600 text-white rounded px-4 py-2">+ New workflow</button>
      </form>
      <p v-if="error" class="text-sm text-red-600">{{ error }}</p>
      <ul class="divide-y bg-white rounded shadow">
        <li v-for="wf in store.workflows" :key="wf.id" class="flex justify-between items-center px-4 py-3">
          <router-link :to="{ name: 'workflow-editor', params: { id: wf.id } }" class="text-blue-600">{{ wf.name }}</router-link>
          <div class="flex items-center gap-3">
            <label class="text-sm flex items-center gap-1">
              <input
                type="checkbox"
                :checked="wf.active"
                @change="setActive(wf.id, ($event.target as HTMLInputElement).checked)"
              />
              Active
            </label>
            <button class="text-sm text-red-600" @click="removeWorkflow(wf.id)">Delete</button>
          </div>
        </li>
      </ul>
      <p v-if="!store.loading && store.workflows.length === 0" class="text-gray-400 text-center py-8">No workflows yet.</p>
    </div>
  </main>
</template>
