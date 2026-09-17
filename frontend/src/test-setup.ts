import { beforeEach, afterEach, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

// Node >= 26 ships its own experimental global `localStorage`. Because that
// global already exists, vitest's jsdom environment never copies jsdom's
// simulated one onto the global object: its `populateGlobal` drops every
// jsdom key that is already `in global` and isn't on its own allow-list, and
// `localStorage` is not on that list. Node's own global then resolves to
// `undefined` (with an ExperimentalWarning) unless the process was started
// with `--localstorage-file`, so code reading the bare global (api/client.ts)
// fails with "Cannot read properties of undefined". `window.localStorage` is
// no escape hatch either — vitest points `window` at `globalThis`.
//
// The previous workaround was `NODE_OPTIONS=--no-experimental-webstorage` in
// package.json's test script, which is POSIX-only shell syntax and makes
// `node` itself refuse to start on any version predating that flag. Instead,
// install a minimal in-memory Storage here, once, so `npm test` is a plain
// `vitest run` on every platform and Node version.
class MemoryStorage implements Storage {
  private entries = new Map<string, string>()

  get length(): number {
    return this.entries.size
  }
  key(index: number): string | null {
    return [...this.entries.keys()][index] ?? null
  }
  getItem(key: string): string | null {
    return this.entries.get(String(key)) ?? null
  }
  setItem(key: string, value: string): void {
    this.entries.set(String(key), String(value))
  }
  removeItem(key: string): void {
    this.entries.delete(String(key))
  }
  clear(): void {
    this.entries.clear()
  }
  [name: string]: unknown
}

Object.defineProperty(globalThis, 'localStorage', {
  value: new MemoryStorage(),
  configurable: true,
  writable: true,
})

// Ensure every test starts with an active Pinia instance, so components that
// call useXStore() during setup() (e.g. CredentialPicker inside
// NodeConfigPanel) work even in spec files that don't set up Pinia
// themselves. Spec files that call setActivePinia(createPinia()) in their
// own beforeEach still work as before — that call simply runs after this
// one and takes precedence for that file.
beforeEach(() => {
  setActivePinia(createPinia())

  // Provide a harmless default fetch stub so components that fire
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
