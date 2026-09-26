import { defineStore } from 'pinia'
import { api } from '../api/client'
import type { Workflow } from '../types/domain'

export const useWorkflowsStore = defineStore('workflows', {
  state: () => ({
    workflows: [] as Workflow[],
    loading: false,
  }),
  actions: {
    async fetchAll() {
      this.loading = true
      try {
        this.workflows = await api.get<Workflow[]>('/rest/r8r/workflows')
      } finally {
        this.loading = false
      }
    },
    async create(name: string): Promise<Workflow> {
      const workflow = await api.post<Workflow>('/rest/r8r/workflows', { name, nodes: [], connections: [] })
      this.workflows.push(workflow)
      return workflow
    },
    async remove(id: string) {
      await api.delete(`/rest/r8r/workflows/${id}`)
      this.workflows = this.workflows.filter((w) => w.id !== id)
    },
    async setActive(id: string, active: boolean) {
      const updated = await api.patch<Workflow>(`/rest/r8r/workflows/${id}/active`, { active })
      const idx = this.workflows.findIndex((w) => w.id === id)
      if (idx !== -1) this.workflows[idx] = updated
    },
  },
})
