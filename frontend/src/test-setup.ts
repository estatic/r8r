import { beforeEach, afterEach, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

// Ensure every test starts with an active Pinia instance, so components that
// call useXStore() during setup() (e.g. CredentialPicker inside
// NodeConfigPanel) work even in spec files that don't set up Pinia
// themselves. Spec files that call setActivePinia(createPinia()) in their
// own beforeEach still work as before — that call simply runs after this
// one and takes precedence for that file.
beforeEach(() => {
  setActivePinia(createPinia())

  // Likewise, provide a harmless default fetch stub so components that fire
  // off a background request on mount (e.g. CredentialPicker's fetchAll())
  // don't produce real network calls / unhandled rejections in spec files
  // that never expected such a request and don't stub fetch themselves.
  // Spec files that stub fetch explicitly (in their own beforeEach or test
  // body) simply override this default, exactly as before.
  vi.stubGlobal(
    'fetch',
    vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => [],
    }),
  )
})

afterEach(() => {
  vi.unstubAllGlobals()
})
