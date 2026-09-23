<script setup lang="ts">
import { ref } from 'vue'
import { useRouter } from 'vue-router'
import { useAuthStore } from '../stores/auth'
import { loginErrorMessage } from '../api/authErrors'

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
    await auth.login(email.value, password.value)
    router.push({ name: 'workflows' })
  } catch (e) {
    error.value = loginErrorMessage(e)
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <main class="min-h-screen bg-gray-50 flex items-center justify-center">
    <form class="bg-white shadow rounded p-8 w-80 space-y-4" @submit.prevent="onSubmit">
      <h1 class="text-xl font-semibold text-gray-800">Log in to r8r</h1>
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
        {{ submitting ? 'Logging in…' : 'Log in' }}
      </button>
      <router-link to="/register" class="block text-sm text-blue-600 text-center">Need an account? Register</router-link>
    </form>
  </main>
</template>
