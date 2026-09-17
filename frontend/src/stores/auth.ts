import { defineStore } from 'pinia'
import { api, getToken, setToken, clearToken } from '../api/client'

export const useAuthStore = defineStore('auth', {
  state: () => ({
    token: getToken() as string | null,
  }),
  getters: {
    isAuthenticated: (state) => state.token !== null,
  },
  actions: {
    async login(email: string, password: string) {
      const response = await api.post<{ token: string }>('/rest/auth/login', { email, password })
      this.token = response.token
      setToken(response.token)
    },
    async register(email: string, password: string) {
      const response = await api.post<{ token: string }>('/rest/auth/register', { email, password })
      this.token = response.token
      setToken(response.token)
    },
    logout() {
      this.token = null
      clearToken()
    },
  },
})
