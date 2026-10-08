// Test-only preload for checking scenarios against n8n: points SDK clients
// whose API host n8n's nodes can't change at the scenario's mock, using
// the same environment variables r8r reads for that.
//   https://api.pinecone.io -> PINECONE_CONTROLLER_HOST (Pinecone Vector Store)
//   https://api.cohere.com  -> CO_API_URL (Reranker Cohere)
const targets = [
  ['https://api.pinecone.io', process.env.PINECONE_CONTROLLER_HOST],
  ['https://api.cohere.com', process.env.CO_API_URL],
].filter(([, target]) => target);
if (targets.length > 0) {
  const orig = globalThis.fetch;
  globalThis.fetch = (input, init) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    for (const [prefix, target] of targets) {
      if (url.startsWith(prefix)) {
        const rewritten = target.replace(/\/$/, '') + url.slice(prefix.length);
        input = typeof input === 'string' || input instanceof URL ? rewritten : new Request(rewritten, input);
        break;
      }
    }
    return orig(input, init);
  };
}
