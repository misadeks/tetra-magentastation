#!/bin/sh
# Minimal supervisor wrapper for the TETRA MagentaStation MS stack, as an
# alternative to systemd (NON-STANDARD, Plane B management interface).
#
# The MS management interface applies a staged configuration by having the stack
# de-register gracefully and then exit with code 75 (EXIT_RESTART). This loop
# respawns the process on exit code 75 so it reloads the new configuration, and
# exits otherwise (e.g. 0 for a clean Ctrl+C shutdown, non-zero/non-75 for a
# genuine failure).
#
# Uses chrt/taskset when installed to control scheduling and CPU affinity.
# Scheduling or privilege failures are reported by the launch command.
#
# Environment knobs (all optional):
#   MAGENTASTATION_BIN   path to magentastation (default /usr/local/bin/magentastation)
#   RT_PRIO           real-time FIFO priority, 1..99 (default 73)
#   CPU_AFFINITY      CPU list for taskset, e.g. "2,3" (default empty = no pin)
#
# Usage: magentastation-supervisor.sh /path/to/config-ms.toml

set -eu

BIN="${MAGENTASTATION_BIN:-/usr/local/bin/magentastation}"
CONFIG="${1:?usage: $0 <config.toml>}"
EXIT_RESTART=75
RT_PRIO="${RT_PRIO:-73}"
CPU_AFFINITY="${CPU_AFFINITY:-}"

# Default log verbosity: quiet `info` for steady-state operation. Override on
# demand for troubleshooting, e.g. `RUST_LOG=debug` or `RUST_LOG=trace`, or
# target one module: `RUST_LOG=tetra_entities::mm=debug`.
export RUST_LOG="${RUST_LOG:-info}"

# Build the launch prefix from whatever scheduling tools are available. Each is
# optional and degrades gracefully so the supervisor still works on a minimal
# system or without the privilege to set RT priority / affinity.
PREFIX=""
if [ -n "$RT_PRIO" ] && command -v chrt >/dev/null 2>&1; then
    PREFIX="chrt -f $RT_PRIO"
fi
if [ -n "$CPU_AFFINITY" ] && command -v taskset >/dev/null 2>&1; then
    PREFIX="taskset -c $CPU_AFFINITY $PREFIX"
fi

while true; do
    set +e
    # shellcheck disable=SC2086 # PREFIX is an intentional word-split command prefix.
    $PREFIX "$BIN" "$CONFIG"
    code=$?
    set -e
    if [ "$code" -eq "$EXIT_RESTART" ]; then
        echo "magentastation: config-apply restart requested (exit $code); respawning..." >&2
        continue
    fi
    echo "magentastation: exited with code $code; not restarting." >&2
    exit "$code"
done
