import { describe, it, expect, vi } from 'vitest'
import { loginErrorMessage, registerErrorMessage } from './authErrors'
import { ApiError } from './client'

describe('loginErrorMessage', () => {
  it('reports bad credentials only for a 401', () => {
    expect(loginErrorMessage(new ApiError(401, 'unauthorized'))).toBe('Invalid email or password.')
  })

  it('reports any other HTTP failure as a server error with its status', () => {
    expect(loginErrorMessage(new ApiError(500, ''))).toBe('Server error (500). Check the r8r terminal for details.')
    expect(loginErrorMessage(new ApiError(502, 'bad gateway'))).toBe('Server error (502). Check the r8r terminal for details.')
  })

  it('reports a request that never got a response as unreachable, and logs the cause', () => {
    const spy = vi.spyOn(console, 'error').mockImplementation(() => {})
    const cause = new TypeError('Failed to fetch')
    expect(loginErrorMessage(cause)).toBe("Can't reach the r8r server. Is it running? (Failed to fetch)")
    expect(spy).toHaveBeenCalledWith('login failed:', cause)
    spy.mockRestore()
  })
})

describe('registerErrorMessage', () => {
  it('keeps the specific 409 and 403 messages', () => {
    expect(registerErrorMessage(new ApiError(409, ''))).toBe('That email is already registered.')
    expect(registerErrorMessage(new ApiError(403, ''))).toBe('Registration is closed. Ask an existing user to invite you.')
  })

  it('reports other failures the same way as login', () => {
    vi.spyOn(console, 'error').mockImplementation(() => {})
    expect(registerErrorMessage(new ApiError(500, ''))).toBe('Server error (500). Check the r8r terminal for details.')
    expect(registerErrorMessage(new TypeError('Failed to fetch'))).toBe("Can't reach the r8r server. Is it running? (Failed to fetch)")
  })
})
