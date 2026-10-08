# Demo: local RAG with r8r and Ollama

A fictional company handbook (Northwind Robotics) is embedded into r8r's
Simple Vector Store. A Q&A chain and an AI agent then answer questions from
it. Everything runs on your machine: Ollama serves both the chat model and
the embeddings through its OpenAI-compatible API, so you need no API keys.
The workflows are plain n8n JSON, so they import into n8n unchanged.

## Run it

```sh
ollama pull llama3.1 && ollama pull nomic-embed-text   # once
demo/rag/run.sh
```

The script builds r8r in release mode and starts a fresh instance in
`demo/rag/.data` on port 5688. It imports one credential and three active
workflows, then runs the story below and keeps the server up until you press
Ctrl-C.

Settings you can override: `PORT`, `OLLAMA_URL`, `CHAT_MODEL` (needs tool
support for the agent, e.g. `qwen3.8`), `EMBED_MODEL`, and `R8R_BIN` to skip
the build.

## The story (about 5 minutes)

1. **Ingest**, at `POST /webhook/rag/ingest`, with the body taken from
   `knowledge.json`. The flow is Webhook → Split Out → Simple Vector Store
   (insert). The store has two sub-nodes: OpenAI Embeddings, pointed at
   Ollama, and a Default Data Loader that takes `$json.text` as the content
   and `topic` as metadata. It returns `{"inserted": 8}`.
2. **Ask**, at `POST /webhook/rag/ask`. A Question and Answer Chain uses a
   Vector Store Retriever (top 3) over the same store.
   *"Who descales the espresso machine, and when?"* → *"Bertha … every
   Tuesday at 8:00 by whoever lost the previous Friday's table-football
   match."* The model can't know this; it comes from the retrieved documents.
3. **Agent**, at `POST /webhook/rag/agent`. An AI Agent gets the store as a
   tool (`retrieve-as-tool`) and decides for itself when to search.
   *"Can I deploy to production on Friday afternoon?"* → no: deploys run
   Monday to Thursday, 10:00–15:00. Then ask something off-handbook, like
   *"What is the parking policy?"*, and it says the handbook has nothing on
   that.
4. **Show it in the editor** at <http://localhost:5688> (log in as
   `demo@r8r.local` / `DemoPassw0rd`). Open the three workflows to see the
   sub-node wiring, then open **Executions** to step through each run,
   including the agent's tool call and the retrieved documents.

More questions that work well: *"What does the Heron-7 do when its battery
is low?"*, *"How many vacation days do I get?"*, *"What do I do if I lose
my laptop?"*. To add your own facts, post `{"docs":[{"topic":"…","text":"…"}]}`
to the ingest webhook. That call clears the store first, so send the whole set.

## Notes

- The Simple Vector Store lives in the server's memory and is shared
  instance-wide by its key, as in n8n. Restarting r8r empties it; run the
  ingest again.
- Ollama is on localhost, so the script sets
  `R8R_SSRF_ALLOWED_HOSTS=localhost,127.0.0.1`. Without it, r8r's SSRF guard
  blocks the calls.
- To use real OpenAI instead, edit the credential to your API key with no
  `url`, and set `CHAT_MODEL=gpt-4o-mini EMBED_MODEL=text-embedding-3-small`.
