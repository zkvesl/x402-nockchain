#!/usr/bin/env bash
# examples/demo.sh — Phase-4/5 end-to-end demo orchestrator.
#
# Usage:
#   bash examples/demo.sh                    # stub mode, no fakenet required
#   bash examples/demo.sh --grpc             # path-2A envelope-only at :9090
#   bash examples/demo.sh --grpc --endpoint http://localhost:9090
#   bash examples/demo.sh --grpc --path2b    # path-2B (client-assembled signed tx)
#
# In --grpc mode, requires a reachable Nockchain public gRPC endpoint.
# Start one with the operator harness in vesl-agent (proprietary):
#   cd ../../vesl-agent && bash harness/fakenet/setup.sh
# (Or, for a bare hub+miner without funded UTXOs, use the upstream
# hull-llm harness directly: `bash ../hull-llm/scripts/fakenet-harness.sh start`.)
#
# `--path2b` is the Phase-5B operator entry point for row 1 of the
# fakenet validation matrix. It exercises the wallet-client end-to-end:
# the demo binary boots a real wallet kernel, the client assembles a
# signed RawTx, and the facilitator submits it via the gRPC chain
# client. Requires `X402_FAKENET_SIGNING_KEY` to be exported (8 Belt
# values) and the harness setup.sh to have funded the matching PKH.
#
# Exits 0 on a full demo success; non-zero on setup/runtime failure.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

MODE="stub"
GRPC_ENDPOINT="http://localhost:9090"
PATH2B="0"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --grpc)
            MODE="grpc"
            shift
            ;;
        --endpoint)
            GRPC_ENDPOINT="$2"
            shift 2
            ;;
        --stub)
            MODE="stub"
            shift
            ;;
        --path2b)
            PATH2B="1"
            shift
            ;;
        -h|--help)
            sed -n '2,22p' "$0"
            exit 0
            ;;
        *)
            echo "unknown flag: $1" >&2
            exit 2
            ;;
    esac
done

if [[ "$PATH2B" == "1" && "$MODE" != "grpc" ]]; then
    echo "error: --path2b requires --grpc" >&2
    exit 2
fi

cd "$REPO_ROOT"

if ! command -v cargo >/dev/null 2>&1; then
    echo "error: cargo not found on PATH" >&2
    exit 1
fi

if [[ "$MODE" == "grpc" ]]; then
    if [[ "$PATH2B" == "1" ]]; then
        echo "[demo.sh] mode=path2b endpoint=$GRPC_ENDPOINT"
    else
        echo "[demo.sh] mode=grpc endpoint=$GRPC_ENDPOINT"
    fi
    # Probe that *something* is listening. If the port is dead, bail early
    # with a clear pointer rather than letting the tonic connect error
    # bubble up from the Rust side.
    # gRPC speaks HTTP/2 — curl over HTTP/1.1 will often error even when
    # the port is healthy. Use bash's /dev/tcp special form for a clean
    # connect probe (note: it expects host/port slash-separated).
    HOST_PORT="${GRPC_ENDPOINT#http://}"
    PROBE_HOST="${HOST_PORT%:*}"
    PROBE_PORT="${HOST_PORT#*:}"
    if ! (exec 3<>/dev/tcp/"$PROBE_HOST"/"$PROBE_PORT") 2>/dev/null; then
        echo "error: nothing listening at $GRPC_ENDPOINT" >&2
        echo "  hint: start fakenet via the operator harness:" >&2
        echo "    cd $REPO_ROOT/../../vesl-agent && bash harness/fakenet/setup.sh start" >&2
        exit 1
    fi
    if [[ "$PATH2B" == "1" ]]; then
        if [[ -z "${X402_FAKENET_SIGNING_KEY:-}" ]]; then
            echo "error: --path2b needs X402_FAKENET_SIGNING_KEY (8 Belt values)" >&2
            echo "  hint: export from vesl-agent/harness/fakenet/funded-keys/<key>.t8" >&2
            exit 2
        fi
        # path-2B compiles in the wallet-client crate's optional
        # `kernels-open-wallet` dep; pull it in via the e2e_demo
        # `path2b` feature.
        exec cargo run -p e2e_demo --features path2b --bin demo_full \
            -- --mode path2b --grpc-endpoint "$GRPC_ENDPOINT"
    fi
    exec cargo run -p e2e_demo --bin demo_full -- --mode grpc --grpc-endpoint "$GRPC_ENDPOINT"
else
    echo "[demo.sh] mode=stub (fakenet not required)"
    exec cargo run -p e2e_demo --bin demo_full -- --mode stub
fi
