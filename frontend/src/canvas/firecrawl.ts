/** The Web Search (Firecrawl) node's form, and its stored parameters. */

export interface FirecrawlForm {
  operation: 'search' | 'scrape'
  query: string
  limit: string
  sources: string[]
  timeRange: string
  country: string
  includeDomains: string
  excludeDomains: string
  scrapeResults: boolean
  url: string
  formats: string[]
  onlyMainContent: boolean
  /** Requests a minute with this API key (empty = no limit). */
  maxPerMinute: string
}

export const SEARCH_SOURCES = [
  { value: 'web', label: 'Web' },
  { value: 'news', label: 'News' },
  { value: 'images', label: 'Images' },
]
export const TIME_RANGES = [
  { value: '', label: 'Any time' },
  { value: 'qdr:h', label: 'Past hour' },
  { value: 'qdr:d', label: 'Past day' },
  { value: 'qdr:w', label: 'Past week' },
  { value: 'qdr:m', label: 'Past month' },
  { value: 'qdr:y', label: 'Past year' },
]
export const SCRAPE_FORMATS = [
  { value: 'markdown', label: 'Markdown' },
  { value: 'summary', label: 'Summary' },
  { value: 'html', label: 'HTML' },
  { value: 'links', label: 'Links' },
  { value: 'images', label: 'Images' },
]

/** The keys the form owns (rewritten on Apply). */
export const FIRECRAWL_KEYS = [
  'operation', 'query', 'limit', 'sources', 'time_range', 'country', 'include_domains', 'exclude_domains', 'scrape_results', 'url', 'formats', 'only_main_content', 'max_requests_per_minute',
]

const str = (v: unknown) => (v === undefined || v === null ? '' : String(v))
const list = (v: unknown, fallback: string[]) => (Array.isArray(v) && v.length > 0 ? v.filter((x): x is string => typeof x === 'string') : fallback)
const domains = (v: unknown) => (Array.isArray(v) ? v.join(', ') : str(v))

export function loadFirecrawl(p: Record<string, unknown>): FirecrawlForm {
  return {
    operation: p.operation === 'scrape' ? 'scrape' : 'search',
    query: str(p.query),
    limit: p.limit === undefined ? '5' : str(p.limit),
    sources: list(p.sources, ['web']),
    timeRange: str(p.time_range),
    country: str(p.country),
    includeDomains: domains(p.include_domains),
    excludeDomains: domains(p.exclude_domains),
    scrapeResults: p.scrape_results === true,
    url: str(p.url),
    formats: list(p.formats, ['markdown']),
    onlyMainContent: p.only_main_content !== false,
    maxPerMinute: str(p.max_requests_per_minute),
  }
}

const splitDomains = (s: string) => s.split(',').map((d) => d.trim()).filter(Boolean)

/** The node's parameters, or why the form can't be saved. */
export function buildFirecrawl(f: FirecrawlForm): { fields: Record<string, unknown> } | { error: string } {
  const built = buildOperation(f)
  if ('error' in built || !f.maxPerMinute.trim()) return built
  const n = Number(f.maxPerMinute)
  if (!Number.isInteger(n) || n < 1 || n > 6000) return { error: 'Max requests per minute must be a whole number from 1 to 6000.' }
  return { fields: { ...built.fields, max_requests_per_minute: n } }
}

function buildOperation(f: FirecrawlForm): { fields: Record<string, unknown> } | { error: string } {
  if (f.operation === 'scrape') {
    if (!f.url.trim()) return { error: 'Enter the URL to read.' }
    if (f.formats.length === 0) return { error: 'Choose at least one format.' }
    const fields: Record<string, unknown> = { operation: 'scrape', url: f.url.trim(), formats: [...f.formats] }
    if (!f.onlyMainContent) fields.only_main_content = false
    return { fields }
  }
  if (!f.query.trim()) return { error: 'Enter a search query.' }
  if (f.sources.length === 0) return { error: 'Choose at least one source.' }
  // A number, or an expression that gives one.
  const limit = /^\d+$/.test(f.limit.trim()) ? Number(f.limit.trim()) : f.limit.trim()
  if (typeof limit === 'number' && (limit < 1 || limit > 100)) return { error: 'Results must be between 1 and 100.' }
  const include = splitDomains(f.includeDomains)
  const exclude = splitDomains(f.excludeDomains)
  if (include.length > 0 && exclude.length > 0) return { error: 'Use either "Only these sites" or "Skip these sites", not both.' }
  const fields: Record<string, unknown> = { operation: 'search', query: f.query, limit: limit === '' ? 5 : limit, sources: [...f.sources] }
  if (f.timeRange) fields.time_range = f.timeRange
  if (f.country.trim()) fields.country = f.country.trim().toUpperCase()
  if (include.length > 0) fields.include_domains = include
  if (exclude.length > 0) fields.exclude_domains = exclude
  if (f.scrapeResults) fields.scrape_results = true
  return { fields }
}
