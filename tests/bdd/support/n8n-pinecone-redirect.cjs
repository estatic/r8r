// Test-only: point the Pinecone client's fixed controller host at the
// scenario's mock (n8n passes a config object, so the client ignores
// PINECONE_CONTROLLER_HOST itself).
const target = process.env.PINECONE_CONTROLLER_HOST;
if (target) {
  const PREFIX = 'https://api.pinecone.io';
  const orig = globalThis.fetch;
  globalThis.fetch = (input, init) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    if (url.startsWith(PREFIX)) {
      const rewritten = target.replace(/\/$/, '') + url.slice(PREFIX.length);
      input = typeof input === 'string' || input instanceof URL ? rewritten : new Request(rewritten, input);
    }
    return orig(input, init);
  };
}
