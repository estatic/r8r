#!/usr/bin/env bash
# Local RAG demo: r8r + Ollama, no API keys. See demo/rag/README.md.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
PORT="${PORT:-5688}"
OLLAMA_URL="${OLLAMA_URL:-http://localhost:11434}"
CHAT_MODEL="${CHAT_MODEL:-llama3.1}"
EMBED_MODEL="${EMBED_MODEL:-nomic-embed-text}"
DATA="$HERE/.data"
BASE="http://localhost:$PORT"
OWNER_EMAIL="demo@r8r.local"
OWNER_PASSWORD="DemoPassw0rd"

say() { printf '\n\033[1;36m▶ %s\033[0m\n' "$*"; }

# ---- prerequisites ---------------------------------------------------------
say "Checking Ollama at $OLLAMA_URL"
tags="$(curl -sf "$OLLAMA_URL/api/tags")" || { echo "Ollama is not reachable at $OLLAMA_URL (start it with: ollama serve)"; exit 1; }
for m in "$CHAT_MODEL" "$EMBED_MODEL"; do
  grep -q "\"name\":\"$m[:\"]" <<<"$tags" || { echo "Model $m is missing — run: ollama pull $m"; exit 1; }
done
echo "chat: $CHAT_MODEL, embeddings: $EMBED_MODEL"

if [[ -z "${R8R_BIN:-}" ]]; then
  say "Building r8r (release)"
  cargo build --release --manifest-path "$ROOT/Cargo.toml" --quiet
  R8R_BIN="$ROOT/target/release/r8r"
fi

# ---- fresh instance --------------------------------------------------------
rm -rf "$DATA" && mkdir -p "$DATA"
export N8N_USER_FOLDER="$DATA" HOME="$DATA"
export N8N_ENCRYPTION_KEY="r8r-rag-demo-key"
export N8N_DIAGNOSTICS_ENABLED=false N8N_LOG_LEVEL="${N8N_LOG_LEVEL:-info}" PORT
# Ollama is on localhost, which the SSRF guard blocks unless allowed.
export R8R_SSRF_ALLOWED_HOSTS="localhost,127.0.0.1"

say "Importing the Ollama credential and the three demo workflows"
sed "s#http://localhost:11434#$OLLAMA_URL#" "$HERE/credentials.json" > "$DATA/credentials.json"
sed -e "s#\"llama3.1\"#\"$CHAT_MODEL\"#" -e "s#\"nomic-embed-text\"#\"$EMBED_MODEL\"#" "$HERE/workflows.json" > "$DATA/workflows.json"
(cd "$DATA" && "$R8R_BIN" import:credentials --input="$DATA/credentials.json")
(cd "$DATA" && "$R8R_BIN" import:workflow --input="$DATA/workflows.json")

say "Starting r8r on $BASE (log: $DATA/r8r.log)"
(cd "$DATA" && exec "$R8R_BIN" start) >"$DATA/r8r.log" 2>&1 &
SERVER=$!
trap 'kill $SERVER 2>/dev/null; wait $SERVER 2>/dev/null' EXIT
for _ in $(seq 1 100); do curl -sf "$BASE/healthz" >/dev/null && break; sleep 0.2; done
curl -sf "$BASE/healthz" >/dev/null || { echo "r8r did not start:"; tail -20 "$DATA/r8r.log"; exit 1; }

curl -sf -X POST "$BASE/rest/owner/setup" -H 'content-type: application/json' \
  -d "{\"email\":\"$OWNER_EMAIL\",\"firstName\":\"Demo\",\"lastName\":\"Owner\",\"password\":\"$OWNER_PASSWORD\"}" >/dev/null \
  && echo "Editor login: $OWNER_EMAIL / $OWNER_PASSWORD"

post() { curl -s -X POST "$BASE/webhook/$1" -H 'content-type: application/json' -d "$2"; echo; }

say "1 · Ingest: embed the handbook into the Simple Vector Store"
post rag/ingest "@$HERE/knowledge.json"

say "2 · Ask (Q&A chain): Who descales the espresso machine, and when?"
post rag/ask '{"question": "Who descales the espresso machine, and when?"}'

say "3 · Agent with the handbook as a tool: Can I deploy on Friday afternoon?"
post rag/agent '{"question": "Can I deploy to production on Friday afternoon?"}'

cat <<EOF

$(printf '\033[1;32m')r8r is running at $BASE — Ctrl-C to stop.$(printf '\033[0m')
  Editor:  $BASE  ($OWNER_EMAIL / $OWNER_PASSWORD)
  Ask:     curl -s -X POST $BASE/webhook/rag/ask   -H 'content-type: application/json' -d '{"question":"What does the Heron-7 do?"}'
  Agent:   curl -s -X POST $BASE/webhook/rag/agent -H 'content-type: application/json' -d '{"question":"How many vacation days do I get?"}'
  Ingest:  curl -s -X POST $BASE/webhook/rag/ingest -H 'content-type: application/json' -d '{"docs":[{"topic":"x","text":"..."}]}'
EOF
wait $SERVER || true
