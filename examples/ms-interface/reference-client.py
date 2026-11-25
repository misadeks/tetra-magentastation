#!/usr/bin/env python3
"""Minimal external-application server for the MagentaStation MS interface.

The historical filename is retained, but this application listens: the stack
connects as a WebSocket client. It performs read-only management discovery and
prints telemetry. No radio commands, config writes or speech are generated.

Install: python -m pip install "websockets>=14"
Run:     python reference-client.py
Then:    cargo run -p magentastation -- example_config/config-offline.toml
"""

import argparse
import asyncio
import json

try:
    from websockets.asyncio.server import serve
except ImportError as error:
    raise SystemExit('Install the example dependency: python -m pip install "websockets>=14"') from error

CONTROL_SUBPROTOCOL = "bluestation-control-v1"
TELEMETRY_SUBPROTOCOL = "bluestation-telemetry-v1"


async def control(connection):
    for handle, command, variant in [
        (1, "GetInterfaceVersion", "InterfaceVersion"),
        (2, "GetState", "State"),
        (3, "GetConfig", "Config"),
    ]:
        payload = {"Management": {command: {"handle": handle}}}
        await connection.send(json.dumps(payload).encode("utf-8"))
        response = json.loads(await asyncio.wait_for(connection.recv(), timeout=15))
        result = response["Management"][variant]
        if result["handle"] != handle:
            raise ValueError(f"Unexpected response handle: {result!r}")
        print(command, json.dumps(result, indent=2), flush=True)
    await connection.wait_closed()


async def telemetry(connection):
    async for payload in connection:
        print("telemetry", json.dumps(json.loads(payload)), flush=True)


async def main(args):
    async with (
        serve(control, "127.0.0.1", args.control_port, subprotocols=[CONTROL_SUBPROTOCOL]),
        serve(telemetry, "127.0.0.1", args.telemetry_port, subprotocols=[TELEMETRY_SUBPROTOCOL]),
    ):
        print(
            f"Listening on loopback: control={args.control_port}, telemetry={args.telemetry_port}",
            flush=True,
        )
        await asyncio.get_running_loop().create_future()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--control-port", type=int, default=9102)
    parser.add_argument("--telemetry-port", type=int, default=9101)
    try:
        asyncio.run(main(parser.parse_args()))
    except KeyboardInterrupt:
        pass
