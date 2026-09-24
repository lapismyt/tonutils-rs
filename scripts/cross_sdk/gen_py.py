#!/usr/bin/env python3
"""Reference wire bytes for cross-SDK TL fixture cases via pytoniq-core.

pytoniq-core serializes the canonical fixture fields through its own TL
engine (its pinned bundled `ton_api.tl`) and compares, or writes, `raw_hex`.

The bundled pytoniq-core schema contains no `quic.*` constructors, so cases
whose `references` do not list `pytoniq-core` are skipped by design; those
cases are covered by the tonutils-go generator and by the schema_audit test.

Usage:
    python3 gen_py.py                 verify raw_hex against pytoniq-core
    python3 gen_py.py --write         regenerate raw_hex in the fixture files
"""

import argparse
import glob
import json
import os
import sys

import pytoniq_core
from pytoniq_core.tl import TlGenerator

SCHEMA = os.path.join(os.path.dirname(pytoniq_core.__file__), "tl", "schemas", "ton_api.tl")
DEFAULT_FIXTURES = os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "..", "..", "fixtures", "cross_sdk"
)


def ip_wire(dotted: str) -> int:
    """Dotted IPv4 to the wire int used by adnl.address.udp.

    The wire int value equals the global-config style value (big-endian
    octet interpretation: 127.0.0.1 = 0x7f000001); pytoniq-core then writes
    it as signed little-endian, producing the same bytes as tonutils-go's
    reversed-network-order write. Addresses with the high bit set become
    negative i32 values under pytoniq's signed `int` encoding.
    """
    value = int.from_bytes(bytes(int(part) for part in dotted.split(".")), "big")
    return value - 2**32 if value >= 2**31 else value


def pubkey(key_hex: str) -> dict:
    return {"@type": "pub.ed25519", "key": key_hex}


def addr_list(value: dict) -> dict:
    addrs = []
    for addr in value["addrs"]:
        if "udp" in addr:
            udp = addr["udp"]
            addrs.append(
                {"@type": "adnl.address.udp", "ip": ip_wire(udp["ip"]), "port": udp["port"]}
            )
        elif "quic" in addr:
            quic_addr = addr["quic"]
            addrs.append(
                {
                    "@type": "adnl.address.quic",
                    "ip": ip_wire(quic_addr["ip"]),
                    "port": quic_addr["port"],
                }
            )
        else:
            raise ValueError(f"unsupported address kind: {addr!r}")
    return {
        "addrs": addrs,
        "version": value["version"],
        "reinit_date": value["reinit_date"],
        "priority": value["priority"],
        "expire_at": value["expire_at"],
    }


def dht_node(node: dict) -> dict:
    return {
        "id": pubkey(node["id"]),
        "addr_list": addr_list(node["addr_list"]),
        "version": node["version"],
        "signature": bytes.fromhex(node["signature"]),
    }


UPDATE_RULES = {
    "signature": "dht.updateRule.signature",
    "anybody": "dht.updateRule.anybody",
    "overlayNodes": "dht.updateRule.overlayNodes",
}


def dht_value(value: dict) -> dict:
    key_desc = value["key_description"]
    key = key_desc["key"]
    return {
        "key": {
            "key": {
                "id": key["id"],
                "name": bytes.fromhex(key["name"]),
                "idx": key["idx"],
            },
            "id": pubkey(key_desc["id"]),
            "update_rule": {"@type": UPDATE_RULES[key_desc["update_rule"]]},
            "signature": bytes.fromhex(key_desc["signature"]),
        },
        "value": bytes.fromhex(value["value"]),
        "ttl": value["ttl"],
        "signature": bytes.fromhex(value["signature"]),
    }


def overlay_node(node: dict) -> dict:
    return {
        "id": pubkey(node["id"]),
        "overlay": node["overlay"],
        "version": node["version"],
        "signature": bytes.fromhex(node["signature"]),
    }


def payload(constructor: str, fields: dict) -> dict:
    if constructor == "dht.findNode":
        return {"key": fields["key"], "k": fields["k"]}
    if constructor == "dht.findValue":
        return {"key": fields["key"], "k": fields["k"]}
    if constructor == "dht.ping":
        return {"random_id": fields["random_id"]}
    if constructor == "dht.getSignedAddressList":
        return {}
    if constructor == "dht.nodes":
        return {"nodes": [dht_node(node) for node in fields["nodes"]]}
    if constructor == "dht.node":
        return dht_node(fields)
    if constructor == "dht.valueFound":
        return {"value": dht_value(fields["value"])}
    if constructor == "dht.valueNotFound":
        return {"nodes": {"nodes": [dht_node(node) for node in fields["nodes"]]}}
    if constructor == "overlay.getRandomPeers":
        return {"peers": {"nodes": [overlay_node(node) for node in fields["peers"]]}}
    if constructor == "overlay.ping":
        return {}
    if constructor == "overlay.query":
        return {"overlay": fields["overlay"]}
    if constructor == "adnl.message.query":
        return {"query_id": fields["query_id"], "query": bytes.fromhex(fields["query"])}
    if constructor == "adnl.message.answer":
        return {"query_id": fields["query_id"], "answer": bytes.fromhex(fields["answer"])}
    if constructor == "adnl.message.nop":
        return {}
    raise KeyError(f"unsupported constructor: {constructor}")


def first_diff(expected_hex: str, got_hex: str) -> str:
    limit = min(len(expected_hex), len(got_hex))
    for i in range(0, limit, 2):
        if expected_hex[i : i + 2] != got_hex[i : i + 2]:
            byte = i // 2
            start = max(0, i - 8)
            return (
                f"first difference at byte {byte}: "
                f"expected ...{expected_hex[start:]}..., got ...{got_hex[start:]}..."
            )
    return f"length differs: expected {len(expected_hex) // 2} bytes, got {len(got_hex) // 2} bytes"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixtures", default=DEFAULT_FIXTURES, help="fixture directory")
    parser.add_argument("--write", action="store_true", help="write raw_hex back into fixtures")
    args = parser.parse_args()

    paths = sorted(
        path
        for path in glob.glob(os.path.join(args.fixtures, "*.json"))
        if os.path.basename(path) != "manifest.json"
    )
    if not paths:
        print(f"gen_py: no fixture files found in {args.fixtures}", file=sys.stderr)
        return 1

    schemas = TlGenerator(SCHEMA).generate()
    total, failed = 0, 0

    for path in paths:
        with open(path, encoding="utf-8") as handle:
            fixture = json.load(handle)
        changed = False
        for case in fixture["cases"]:
            if "pytoniq-core" not in case["references"]:
                print(f"SKIP {case['name']} (not covered by pytoniq-core schema)")
                continue
            total += 1
            try:
                out = schemas.serialize(
                    case["constructor"], payload(case["constructor"], case["fields"]), boxed=True
                )
            except Exception as error:  # noqa: BLE001 - report any engine failure as a case failure
                print(f"FAIL {case['name']}: pytoniq-core serialize: {error}")
                failed += 1
                continue
            got = out.hex()
            if args.write:
                if case["raw_hex"] != got:
                    case["raw_hex"] = got
                    changed = True
                print(f"WROTE {case['name']} ({len(out)} bytes)")
                continue
            if not case["raw_hex"]:
                print(f"FAIL {case['name']}: raw_hex is empty; run with --write to generate it")
                failed += 1
                continue
            if case["raw_hex"] != got:
                print(
                    f"FAIL {case['name']}: pytoniq-core serialization diverges: "
                    f"{first_diff(case['raw_hex'], got)}"
                )
                failed += 1
                continue
            print(f"OK {case['name']} ({len(out)} bytes)")

        if changed:
            with open(path, "w", encoding="utf-8") as handle:
                json.dump(fixture, handle, indent=2)
                handle.write("\n")

    print(f"pytoniq-core: {total} cases, {failed} failed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
