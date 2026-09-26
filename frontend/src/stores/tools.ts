import { defineStore } from 'pinia'
import { api, ApiError } from '../api/client'
import type { Tool } from '../types/domain'

export type ToolBody = Pick<Tool, 'name' | 'description' | 'node_type' | 'argument_schema' | 'parameters'>

/** Workflow names from a tool DELETE 409 body, or null if `e` isn't one. */
export function inUseToolWorkflowNames(e: unknown): string[] | null {
  if (!(e instanceof ApiError) || e.status !== 409) return null
  try {
    const body = JSON.parse(e.message) as { workflows?: { name: string }[] }
    return Array.isArray(body.workflows) ? body.workflows.map((w) => w.name) : null
  } catch {
    return null
  }
}

export const useToolsStore = defineStore('tools', {
  state: () => ({
    tools: [] as Tool[],
    loaded: false,
  }),
  actions: {
    async fetchAll() {
      this.tools = await api.get<Tool[]>('/rest/r8r/tools')
      this.loaded = true
    },
    async get(id: string): Promise<Tool> {
      return api.get<Tool>(`/rest/r8r/tools/${id}`)
    },
    async create(body: ToolBody): Promise<Tool> {
      const tool = await api.post<Tool>('/rest/r8r/tools', body)
      await this.fetchAll()
      return tool
    },
    async update(id: string, patch: Partial<ToolBody>): Promise<Tool> {
      const tool = await api.patch<Tool>(`/rest/r8r/tools/${id}`, patch)
      await this.fetchAll()
      return tool
    },
    async remove(id: string): Promise<void> {
      await api.delete<void>(`/rest/r8r/tools/${id}`)
      await this.fetchAll()
    },
  },
})
