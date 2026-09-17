import { defineStore } from 'pinia'
import { api } from '../api/client'

export const useNodeTypesStore = defineStore('nodeTypes', {
  state: () => ({
    types: [] as string[],
    loaded: false,
  }),
  actions: {
    async fetchAll() {
      if (this.loaded) return
      this.types = await api.get<string[]>('/rest/node-types')
      this.loaded = true
    },
  },
})
