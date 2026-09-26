import { defineStore } from 'pinia'
import { api } from '../api/client'
import type { CredentialTypeSchema } from '../types/domain'

export const useCredentialTypesStore = defineStore('credentialTypes', {
  state: () => ({
    types: [] as CredentialTypeSchema[],
    loaded: false,
  }),
  actions: {
    async fetchAll() {
      if (this.loaded) return
      this.types = await api.get<CredentialTypeSchema[]>('/rest/r8r/credential-types')
      this.loaded = true
    },
  },
})
