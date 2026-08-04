#!/usr/bin/env python3
"""Report agent-fleet quota from the local Agent Quota portal.

Usage:
  quota.py                 human-readable table
  quota.py --json          raw pass-through
  quota.py --gate ID       exit 0 if that account has headroom, 1 if exhausted

`--gate` is the dispatch guard: a lane runs only when its engine has quota, so
an exhausted engine hands its work on instead of failing mid-task.
"""
import argparse
import json
import sys
import urllib.request

URL = "http://192.168.50.63:5171/api/overview"
# Below this remaining-percent a lane is treated as exhausted and handed off.
FLOOR_PERCENT = 8.0


def fetch():
    with urllib.request.urlopen(URL, timeout=20) as r:
        return json.load(r)


def worst_remaining(account):
    """Lowest remaining-percent across an account's limit windows."""
    vals = [
        l.get("remaining_percent")
        for l in (account.get("limits") or [])
        if l.get("remaining_percent") is not None
    ]
    return min(vals) if vals else None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--gate", metavar="ID")
    args = ap.parse_args()

    data = fetch()

    if args.json:
        json.dump(data, sys.stdout, ensure_ascii=False, indent=2)
        return 0

    accounts = {a.get("id"): a for a in data.get("accounts", [])}

    if args.gate:
        a = accounts.get(args.gate)
        if a is None:
            print("unknown account: %s" % args.gate, file=sys.stderr)
            return 2
        if not a.get("authenticated") or a.get("status") != "ok":
            print("BLOCKED %s: unauthenticated or error" % args.gate)
            return 1
        rem = worst_remaining(a)
        if rem is not None and rem < FLOOR_PERCENT:
            print("BLOCKED %s: %.1f%% remaining (floor %.1f%%)"
                  % (args.gate, rem, FLOOR_PERCENT))
            return 1
        print("OK %s: %s%% remaining"
              % (args.gate, "n/a" if rem is None else "%.1f" % rem))
        return 0

    for a in data.get("accounts", []):
        parts = []
        for l in a.get("limits") or []:
            parts.append("%s:%s%% (reset %s)" % (
                l.get("label"), l.get("remaining_percent"), l.get("resets_in")))
        rem = worst_remaining(a)
        flag = "  " if rem is None or rem >= FLOOR_PERCENT else "!!"
        print("%s %-16s %-7s auth=%-5s st=%-6s | %s" % (
            flag, a.get("id"), a.get("service"),
            a.get("authenticated"), a.get("status"), "; ".join(parts) or "-"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
