#!/usr/bin/env bash
#
# Profile 0 devnet demo: 1 single-sequencer node + 1 worker.
#
# Starts a ducp-node sequencer, then runs the beachhead workload against it via
# JSON-RPC (submit -> claim -> execute -> proof -> settle), repeatedly.
#
#   PORT=8650 TASKS=5 scripts/devnet.sh
#
set -euo pipefail
cd "$(dirname "$0")/.."

PORT="${PORT:-8650}"
TASKS="${TASKS:-5}"

echo "==> building node + worker"
cargo build -p ducp-node --bins

echo "==> starting sequencer on 127.0.0.1:${PORT}"
./target/debug/ducp-node --listen "127.0.0.1:${PORT}" &
SEQ_PID=$!
trap 'kill "${SEQ_PID}" 2>/dev/null || true' EXIT

# Wait for the RPC port to accept connections.
for _ in $(seq 1 30); do
  if nc -z 127.0.0.1 "${PORT}" 2>/dev/null; then break; fi
  sleep 0.2
done

echo "==> running worker (${TASKS} tasks)"
./target/debug/ducp-worker --sequencer "http://127.0.0.1:${PORT}" --tasks "${TASKS}"

echo "==> devnet demo complete"
