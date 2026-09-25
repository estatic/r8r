import { defineStore } from 'pinia'
import { api, ApiError } from '../api/client'
import type { CredentialDetail, CredentialSummary } from '../types/domain'

/** Workflow names from a DELETE 409 body, or null if `e` isn't one. */
export function inUseWorkflowNames(e: unknown): string[] | null {
  if (!(e instanceof ApiError) || e.status !== 409) return null
  try {
    const body = JSON.parse(e.message) as { workflows?: { name: string }[]; tools?: { name: string }[] }
    if (!Array.isArray(body.workflows)) return null
    return [...body.workflows.map((w) => w.name), ...(body.tools ?? []).map((t) => `${t.name} (tool)`)]
  } catch {
    return null
  }
}

export const useCredentialsStore = defineStore('credentials', {
  state: () => ({
    credentials: [] as CredentialSummary[],
    loaded: false,
  }),
  actions: {
    async fetchAll() {
      this.credentials = await api.get<CredentialSummary[]>('/rest/credentials')
      this.loaded = true
    },
    async create(name: string, credentialType: string, data: Record<string, unknown>): Promise<CredentialSummary> {
      const summary = await api.post<CredentialSummary>('/rest/credentials', {
        name,
        credential_type: credentialType,
        data,
      })
      const withUsage = { ...summary, used_by: summary.used_by ?? 0 }
      this.credentials.push(withUsage)
      return withUsage
    },
    async get(id: string): Promise<CredentialDetail> {
      return api.get<CredentialDetail>(`/rest/credentials/${id}`)
    },
    async update(id: string, patch: { name?: string; data?: Record<string, unknown> }): Promise<CredentialSummary> {
      const summary = await api.patch<CredentialSummary>(`/rest/credentials/${id}`, patch)
      await this.fetchAll()
      return summary
    },
    async remove(id: string): Promise<void> {
      await api.delete<void>(`/rest/credentials/${id}`)
      await this.fetchAll()
    },
  },
})