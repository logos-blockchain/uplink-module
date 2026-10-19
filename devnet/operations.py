#!/usr/bin/env python3
"""Lists what uplink_module sent for each deployment, from its saved state: registrations,
claims (the epochs each covered and the points it paid) and cash-outs, oldest first.

    python3 devnet/operations.py <path to uplink_state.json>

The module keeps the file in its data folder (Basecamp's, or logoscore's --persistence-path);
`find` it with:
    find ~/Library "$HOME/.local/share" "$HOME/.logoscore" -name uplink_state.json 2>/dev/null
"""
import datetime
import json
import sys

path = sys.argv[1] if len(sys.argv) > 1 else sys.exit(__doc__)
saved = json.load(open(path))
selected = (saved.get("config") or {}).get("programAccount")
for program, deployment in saved.get("deployments", {}).items():
    print(f"program {program}{'  (selected)' if program == selected else ''}")
    operations = sorted(deployment.get("operations", {}).values(), key=lambda op: op.get("createdAtMs", 0))
    for op in operations:
        when = datetime.datetime.fromtimestamp(op.get("createdAtMs", 0) / 1000).strftime("%H:%M:%S")
        detail = op.get("detail") or {}
        what = ", ".join(f"{k}={v}" for k, v in detail.items())
        print(f"  {when}  {op.get('kind'):9} {op.get('status'):10} {what}" + (f"  error={op['error']}" if op.get("error") else ""))
    if not operations:
        print("  (no operations)")
