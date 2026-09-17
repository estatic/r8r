import { defineStore } from 'pinia'
import { api } from '../api/client'
import type { CredentialSummary } from '../types/domain'

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
      this.credentials.push(summary)
      return summary
    },
  },
})
