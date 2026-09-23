import { ApiError } from './client'

/**
 * What went wrong with an auth request, in words that point at the cause:
 * only a 401 means bad credentials; any other status is a server problem;
 * no HTTP response at all means the server couldn't be reached. The raw
 * error goes to the console so it can be inspected in devtools.
 */
function describe(e: unknown, action: string): string {
  if (e instanceof ApiError) {
    return `Server error (${e.status}). Check the r8r terminal for details.`
  }
  console.error(`${action} failed:`, e)
  const detail = e instanceof Error && e.message ? ` (${e.message})` : ''
  return `Can't reach the r8r server. Is it running?${detail}`
}

export function loginErrorMessage(e: unknown): string {
  if (e instanceof ApiError && e.status === 401) return 'Invalid email or password.'
  return describe(e, 'login')
}

export function registerErrorMessage(e: unknown): string {
  if (e instanceof ApiError && e.status === 409) return 'That email is already registered.'
  if (e instanceof ApiError && e.status === 403) return 'Registration is closed. Ask an existing user to invite you.'
  return describe(e, 'register')
}
