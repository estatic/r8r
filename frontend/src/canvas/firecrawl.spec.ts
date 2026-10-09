import { describe, expect, it } from 'vitest'
import { buildFirecrawl, loadFirecrawl } from './firecrawl'

describe('Web Search (Firecrawl) form', () => {
  it('starts as a 5-result web search', () => {
    expect(loadFirecrawl({})).toMatchObject({ operation: 'search', limit: '5', sources: ['web'], formats: ['markdown'], onlyMainContent: true })
  })

  it('round-trips a search with every option', () => {
    const fields = {
      operation: 'search',
      query: '{{ $json.message.text }}',
      limit: 3,
      sources: ['web', 'news'],
      time_range: 'qdr:w',
      country: 'DE',
      include_domains: ['docs.rs', 'github.com'],
      scrape_results: true,
    }
    expect(buildFirecrawl(loadFirecrawl(fields))).toEqual({ fields })
  })

  it('round-trips a scrape', () => {
    const fields = { operation: 'scrape', url: '{{ $json.url }}', formats: ['markdown', 'links'], only_main_content: false }
    expect(buildFirecrawl(loadFirecrawl(fields))).toEqual({ fields })
  })

  it('keeps a rate limit for either operation', () => {
    const fields = { operation: 'scrape', url: 'https://x.example', formats: ['markdown'], max_requests_per_minute: 10 }
    expect(buildFirecrawl(loadFirecrawl(fields))).toEqual({ fields })
    expect(buildFirecrawl({ ...loadFirecrawl({ query: 'q' }), maxPerMinute: '0' })).toEqual({ error: 'Max requests per minute must be a whole number from 1 to 6000.' })
  })

  it('says what is missing', () => {
    expect(buildFirecrawl(loadFirecrawl({}))).toEqual({ error: 'Enter a search query.' })
    expect(buildFirecrawl({ ...loadFirecrawl({ query: 'q' }), limit: '500' })).toEqual({ error: 'Results must be between 1 and 100.' })
    expect(buildFirecrawl({ ...loadFirecrawl({ query: 'q' }), includeDomains: 'a.com', excludeDomains: 'b.com' })).toHaveProperty('error')
    expect(buildFirecrawl(loadFirecrawl({ operation: 'scrape' }))).toEqual({ error: 'Enter the URL to read.' })
  })
})
