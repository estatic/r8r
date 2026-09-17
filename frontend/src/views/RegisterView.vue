<script setup lang="ts">
import { ref } from 'vue'
import { useRouter } from 'vue-router'
import { useAuthStore } from '../stores/auth'
import { ApiError } from '../api/client'

const email = ref('')
const password = ref('')
const error = ref('')
const submitting = ref(false)

const auth = useAuthStore()
const router = useRouter()

async function onSubmit() {
  error.value = ''
  submitting.value = true
  try {
    await auth.register(email.value, password.value)
    router.push({ name: 'workflows' })
  } catch (e) {
    error.value = e instanceof ApiError && e.status === 409 ? 'That email is already registered.' : 'Something went wrong. Try again.'
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <main class="min-h-screen bg-gray-50 flex items-center justify-center">
    <form class="bg-white shadow rounded p-8 w-80 space-y-4" @submit.prevent="onSubmit">
      <h1 class="text-xl font-semibold text-gray-800">Create an r8r account</h1>
      <div>
        <label class="block text-sm text-gray-600 mb-1" for="email">Email</label>
        <input id="email" v-model="email" type="email" required class="w-full border rounded px-3 py-2" />
      </div>
      <div>
        <label class="block text-sm text-gray-600 mb-1" for="password">Password</label>
        <input id="password" v-model="password" type="password" required class="w-full border rounded px-3 py-2" />
      </div>
      <p v-if="error" class="text-sm text-red-600">{{ error }}</p>
      <button type="submit" :disabled="submitting" class="w-full bg-blue-600 text-white rounded py-2 disabled:opacity-50">
        {{ submitting ? 'Creating account…' : 'Register' }}
      </button>
      <router-link to="/login" class="block text-sm text-blue-600 text-center">Already have an account? Log in</router-link>
    </form>
  </main>
</template>
