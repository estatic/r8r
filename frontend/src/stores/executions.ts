import { defineStore } from 'pinia'
import { api } from '../api/client'
import type { Execution } from '../types/domain'

export const useExecutionsStore = defineStore('executions', {
  state: () => ({
    history: [] as Execution[],
  }),
  actions: {
    async fetchHistory(workflowId: string, limit = 50) {
      this.history = await api.get<Execution[]>(`/rest/r8r/workflows/${workflowId}/executions?limit=${limit}`)
    },
  },
})
