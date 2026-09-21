import { defineStore } from 'pinia'
import { api } from '../api/client'
import type { NodeTypeMeta } from '../types/domain'

export const useNodeTypesStore = defineStore('nodeTypes', {
  state: () => ({
    types: [] as NodeTypeMeta[],
    loaded: false,
    portsCache: {} as Record<string, string[]>,
    portsInFlight: {} as Record<string, Promise<string[]>>,
  }),
  actions: {
    async fetchAll() {
      if (this.loaded) return
      this.types = await api.get<NodeTypeMeta[]>('/rest/node-types')
      this.loaded = true
    },
    async portsFor(typeName: string, parameters: Record<string, unknown>): Promise<string[]> {
      const key = `${typeName}:${JSON.stringify(parameters)}`
      const cached = this.portsCache[key]
      if (cached) return cached
      const inFlight = this.portsInFlight[key]
      if (inFlight) return inFlight
      const request = api
        .post<{ output_ports: string[] }>(`/rest/node-types/${encodeURIComponent(typeName)}/output-ports`, { parameters })
        .then((result) => {
          this.portsCache[key] = result.output_ports
          delete this.portsInFlight[key]
          return result.output_ports
        })
        .catch((e) => {
          delete this.portsInFlight[key]
          throw e
        })
      this.portsInFlight[key] = request
      return request
    },
  },
})
