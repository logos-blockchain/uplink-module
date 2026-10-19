#!/usr/bin/env python3
"""Turns an Uplink invitation code into the invitation file the uplink CLI reads
(logos-blockchain/uplink: `uplink accept-invite --participant <id> <file>`):

    python3 devnet/invite-to-cli.py 'uplink-invite-v1:…' > invitation.json

The code is "uplink-invite-v1:" + base64url (no padding) of the inviter's node, then its
participant's nullifier and viewing public keys; the CLI's JSON lists the same bytes.
"""
import base64
import json
import sys

PREFIX = "uplink-invite-v1:"

code = sys.argv[1].strip() if len(sys.argv) > 1 else sys.exit(__doc__)
if not code.startswith(PREFIX):
    sys.exit("not an Uplink invitation code (it starts with " + PREFIX + ")")
body = code[len(PREFIX):]
raw = base64.urlsafe_b64decode(body + "=" * (-len(body) % 4))
if len(raw) != 32 + 32 + 1184:
    sys.exit("wrong length: is the whole code copied?")
print(json.dumps({"parent_node": list(raw[:32]), "npk": list(raw[32:64]), "vpk": list(raw[64:])}))
sys.stderr.write("invitation from node " + raw[:32].hex() + "\n")
