<script setup lang="ts">
import { ref } from 'vue'
import FieldPicker from './FieldPicker.vue'
import { SCRAPE_FORMATS, SEARCH_SOURCES, TIME_RANGES, type FirecrawlForm } from '../canvas/firecrawl'
import type { UpstreamSource } from '../canvas/inputData'

/** Web Search (Firecrawl): search the web, or read one page. */
const form = defineModel<FirecrawlForm>({ required: true })
defineProps<{ sources: UpstreamSource[]; nodeLabels: Record<string, string> }>()

const set = (patch: Partial<FirecrawlForm>) => (form.value = { ...form.value, ...patch })
const toggle = (list: string[], value: string, on: boolean) => (on ? [...list.filter((v) => v !== value), value] : list.filter((v) => v !== value))

const picking = ref<'query' | 'url' | null>(null)
function pick(expression: string) {
  if (picking.value) set({ [picking.value]: expression })
  picking.value = null
}
</script>

<template>
  <fieldset class="border rounded p-2 space-y-2 min-w-0" data-testid="firecrawl-settings">
    <legend class="text-sm text-gray-600 px-1">Web Search (Firecrawl)</legend>
    <label class="block text-xs text-gray-600">
      Operation
      <select
        :value="form.operation"
        aria-label="Operation"
        class="w-full border rounded px-2 py-1 text-sm"
        @change="set({ operation: ($event.target as HTMLSelectElement).value as FirecrawlForm['operation'] })"
      >
        <option value="search">Search the web</option>
        <option value="scrape">Read a web page</option>
      </select>
    </label>

    <template v-if="form.operation === 'search'">
      <div class="text-xs text-gray-600">
        Query
        <div class="flex gap-1">
          <input
            :value="form.query"
            aria-label="Query"
            placeholder="{{ $json.message.text }} or best rust web frameworks"
            class="flex-1 min-w-0 border rounded px-2 py-1 text-sm"
            @input="set({ query: ($event.target as HTMLInputElement).value })"
          />
          <button type="button" data-testid="pick-query" class="text-xs border rounded px-2 bg-white hover:bg-blue-50 shrink-0" @click="picking = picking === 'query' ? null : 'query'">
            Pick ▾
          </button>
        </div>
        <span class="text-[11px] text-gray-400">Operators work: site:, intitle:, filetype:, -word</span>
      </div>
      <FieldPicker v-if="picking === 'query'" :sources="sources" :node-labels="nodeLabels" @pick="pick" />
      <div class="grid grid-cols-2 gap-2">
        <label class="text-xs text-gray-600">
          Results (per source)
          <input :value="form.limit" aria-label="Results" class="w-full border rounded px-2 py-1 text-sm" @input="set({ limit: ($event.target as HTMLInputElement).value })" />
        </label>
        <label class="text-xs text-gray-600">
          Time range
          <select :value="form.timeRange" aria-label="Time range" class="w-full border rounded px-2 py-1 text-sm" @change="set({ timeRange: ($event.target as HTMLSelectElement).value })">
            <option v-for="t in TIME_RANGES" :key="t.value" :value="t.value">{{ t.label }}</option>
          </select>
        </label>
      </div>
      <div class="flex gap-3 text-xs text-gray-600">
        Sources:
        <label v-for="s in SEARCH_SOURCES" :key="s.value" class="flex items-center gap-1">
          <input
            type="checkbox"
            :aria-label="`Source ${s.label}`"
            :checked="form.sources.includes(s.value)"
            @change="set({ sources: toggle(form.sources, s.value, ($event.target as HTMLInputElement).checked) })"
          />
          {{ s.label }}
        </label>
      </div>
      <label class="flex items-center gap-2 text-xs">
        <input type="checkbox" aria-label="Scrape results" :checked="form.scrapeResults" @change="set({ scrapeResults: ($event.target as HTMLInputElement).checked })" />
        Also read each result page (adds its <code>markdown</code>; costs more credits)
      </label>
      <details class="border-t pt-2">
        <summary class="text-xs text-gray-600 cursor-pointer">More options</summary>
        <div class="mt-1.5 space-y-1.5">
          <label class="block text-xs text-gray-600">
            Country (2 letters)
            <input :value="form.country" aria-label="Country" placeholder="US" maxlength="2" class="w-full border rounded px-2 py-1 text-sm" @input="set({ country: ($event.target as HTMLInputElement).value })" />
          </label>
          <label class="block text-xs text-gray-600">
            Only these sites
            <input :value="form.includeDomains" aria-label="Only these sites" placeholder="docs.rs, github.com" class="w-full border rounded px-2 py-1 text-sm" @input="set({ includeDomains: ($event.target as HTMLInputElement).value })" />
          </label>
          <label class="block text-xs text-gray-600">
            Skip these sites
            <input :value="form.excludeDomains" aria-label="Skip these sites" placeholder="pinterest.com" class="w-full border rounded px-2 py-1 text-sm" @input="set({ excludeDomains: ($event.target as HTMLInputElement).value })" />
          </label>
        </div>
      </details>
      <p class="text-xs text-gray-500">Each result becomes an item: <code>title</code>, <code>url</code>, <code>description</code> (news: <code>snippet</code>, <code>date</code>), <code>source</code>.</p>
    </template>

    <template v-else>
      <div class="text-xs text-gray-600">
        URL
        <div class="flex gap-1">
          <input
            :value="form.url"
            aria-label="URL"
            placeholder="https://… or {{ $json.url }}"
            class="flex-1 min-w-0 border rounded px-2 py-1 text-sm"
            @input="set({ url: ($event.target as HTMLInputElement).value })"
          />
          <button type="button" data-testid="pick-url" class="text-xs border rounded px-2 bg-white hover:bg-blue-50 shrink-0" @click="picking = picking === 'url' ? null : 'url'">
            Pick ▾
          </button>
        </div>
      </div>
      <FieldPicker v-if="picking === 'url'" :sources="sources" :node-labels="nodeLabels" @pick="pick" />
      <div class="flex flex-wrap gap-3 text-xs text-gray-600">
        Get:
        <label v-for="f in SCRAPE_FORMATS" :key="f.value" class="flex items-center gap-1">
          <input
            type="checkbox"
            :aria-label="`Format ${f.label}`"
            :checked="form.formats.includes(f.value)"
            @change="set({ formats: toggle(form.formats, f.value, ($event.target as HTMLInputElement).checked) })"
          />
          {{ f.label }}
        </label>
      </div>
      <label class="flex items-center gap-2 text-xs">
        <input type="checkbox" aria-label="Only main content" :checked="form.onlyMainContent" @change="set({ onlyMainContent: ($event.target as HTMLInputElement).checked })" />
        Only the main content (no menus, headers, footers)
      </label>
    </template>

    <label class="block text-xs text-gray-600 border-t pt-2">
      Max requests per minute (empty = no limit)
      <input :value="form.maxPerMinute" aria-label="Max requests per minute" type="number" min="1" max="6000" placeholder="e.g. 10" class="w-full border rounded px-2 py-1 text-sm" @input="set({ maxPerMinute: ($event.target as HTMLInputElement).value })" />
      <span class="text-[11px] text-gray-400">Requests with this API key are spaced evenly (10 a minute = one every 6 s), across every run and agent tool call; the rest wait their turn. A request whose turn is over 10 minutes away fails instead of waiting.</span>
    </label>
  </fieldset>
</template>
