import { ApiError } from './client'

/**
 * `fallback`, plus the server's own explanation when it gave one (r8r
 * answers most refusals with a plain-text reason), else the status code.
 */
export function errorText(e: unknown, fallback: string): string {
  if (!(e instanceof ApiError)) return fallback
  const reason = e.message.trim()
  const useful = reason !== '' && !reason.startsWith('request failed with status') && !reason.startsWith('<') && reason.length <= 500
  return useful ? `${fallback} ${reason}` : `${fallback} (${e.status})`
}
