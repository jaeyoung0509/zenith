#!/usr/bin/env python3
"""Compare private Mole and Zenith read-only scan ledgers without publishing paths.

Mole's clean-list is a rounded *potential* list, not a deletion plan or a
verified free-space delta. This tool never invokes either cleaner or deletes
anything. The default JSON output contains no user paths.
"""

import argparse
from collections import Counter, defaultdict
from decimal import Decimal, InvalidOperation
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess


SIZE_RE = re.compile(r"^(\d+(?:\.\d+)?)\s*(B|KB|MB|GB|TB)$", re.I)
SIZE_MULTIPLIER = {
    "B": 1,
    "KB": 1024,
    "MB": 1024**2,
    "GB": 1024**3,
    "TB": 1024**4,
}


def displayed_bytes(label):
    match = SIZE_RE.fullmatch(label.split(",", 1)[0].strip())
    if not match:
        return None
    try:
        return int(Decimal(match.group(1)) * SIZE_MULTIPLIER[match.group(2).upper()])
    except InvalidOperation:
        return None


def parse_mole_preview(path):
    section = "Uncategorized"
    rows = []
    reported_total = None
    for line in path.read_text().splitlines():
        if line.startswith("=== ") and line.endswith(" ==="):
            section = line[4:-4]
        elif line.startswith("# Potential cleanup: "):
            reported_total = line.removeprefix("# Potential cleanup: ")
        elif line.startswith("/") and "  # " in line:
            candidate, size = line.rsplit("  # ", 1)
            rows.append(
                {
                    "path": os.path.normpath(candidate),
                    "section": section,
                    "displayed_bytes": displayed_bytes(size),
                }
            )
    return rows, reported_total


def path_contains(parent, child):
    return child != parent and child.startswith(parent.rstrip(os.sep) + os.sep)


def owner_bucket(path, home):
    parts = Path(path).parts
    if path_contains(os.path.join(home, "Library", "Caches", "Homebrew"), path) or path == os.path.join(home, "Library", "Caches", "Homebrew"):
        return "homebrew"
    if any(part in ("Chrome", "Chromium", "Firefox", "Brave-Browser", "Arc") for part in parts):
        return "browser"
    if "cargo" in parts or ".cargo" in parts:
        return "cargo"
    if path_contains(os.path.join(home, "Library", "Caches"), path):
        return "user_app_cache"
    if path_contains(os.path.join(home, "Library", "Application Support"), path):
        return "application_support"
    if path_contains(os.path.join(home, "Library", "Logs"), path):
        return "user_log"
    return "other"


def allocated_bytes(path):
    try:
        result = subprocess.run(
            ["du", "-skP", path], capture_output=True, text=True, timeout=20, check=True
        )
        return int(result.stdout.split(maxsplit=1)[0]) * 1024
    except (OSError, ValueError, subprocess.CalledProcessError, subprocess.TimeoutExpired):
        return None


def compare(mole_rows, zenith_rows, home, measure):
    by_bucket = defaultdict(lambda: Counter())
    relation = Counter()
    eligibility = Counter()
    details = []
    for mole in mole_rows:
        path = mole["path"]
        exact = [row for row in zenith_rows if row["path"] == path]
        ancestors = [row for row in zenith_rows if path_contains(row["path"], path)]
        descendants = [row for row in zenith_rows if path_contains(path, row["path"])]
        if exact:
            relationship, related = "exact", exact
        elif ancestors:
            # The closest ancestor is the one whose policy describes this path.
            closest = max(ancestors, key=lambda row: len(row["path"]))
            relationship, related = "zenith_ancestor", [closest]
        elif descendants:
            relationship, related = "zenith_descendants", descendants
        else:
            relationship, related = "mole_only", []
        bucket = owner_bucket(path, home)
        allocated = allocated_bytes(path) if measure else None
        relation[relationship] += 1
        group = by_bucket[bucket]
        group["items"] += 1
        group[relationship + "_items"] += 1
        if mole["displayed_bytes"] is None:
            group["unknown_preview_size_items"] += 1
        else:
            group["rounded_preview_bytes"] += mole["displayed_bytes"]
        if measure:
            if allocated is None:
                group["unmeasured_allocated_items"] += 1
            else:
                group["allocated_bytes_at_audit"] += allocated
        for row in related:
            eligibility[row["eligibility"]] += 1
        details.append(
            {
                **mole,
                "bucket": bucket,
                "relation": relationship,
                "allocated_bytes_at_audit": allocated,
                "zenith": related,
            }
        )
    nested_rows = [
        row
        for row in mole_rows
        if any(path_contains(other["path"], row["path"]) for other in mole_rows)
    ]
    top_level_rows = [row for row in mole_rows if row not in nested_rows]
    overlap_summary = {
        "nested_item_count": len(nested_rows),
        "nested_rounded_preview_bytes": sum(row["displayed_bytes"] or 0 for row in nested_rows),
        "top_level_rounded_preview_bytes": sum(
            row["displayed_bytes"] or 0 for row in top_level_rows
        ),
    }
    return by_bucket, relation, eligibility, details, overlap_summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mole-preview", type=Path, required=True)
    parser.add_argument("--zenith-report", type=Path, required=True)
    parser.add_argument("--measure-allocated", action="store_true")
    parser.add_argument("--private-details", action="store_true", help="Include private paths in stdout")
    args = parser.parse_args()
    mole_rows, reported_total = parse_mole_preview(args.mole_preview)
    zenith = json.loads(args.zenith_report.read_text())
    zenith_rows = zenith.get("private_ledger")
    if not isinstance(zenith_rows, list):
        parser.error("Zenith report requires --full-catalog-read-only --private-ledger")
    home = str(Path.home())
    buckets, relations, eligibility, details, overlap_summary = compare(
        mole_rows, zenith_rows, home, args.measure_allocated
    )
    summary = {
        "mole_preview": {
            "sha256": hashlib.sha256(args.mole_preview.read_bytes()).hexdigest(),
            "reported_potential": reported_total,
            "item_count": len(mole_rows),
            "rounded_display_sum_bytes": sum(row["displayed_bytes"] or 0 for row in mole_rows),
            "structural_overlaps": overlap_summary,
        },
        "zenith_scan": {
            "sha256": hashlib.sha256(args.zenith_report.read_bytes()).hexdigest(),
            "version": zenith["version"],
            "started_at": zenith["started_at"],
            "scope": zenith["scope"],
            "quality": zenith["quality"],
            "observed_bytes": zenith["observed_bytes"],
            "cleanable_bytes": zenith["cleanable_bytes"],
            "selected_bytes": zenith["selected_bytes"],
            "gaps": zenith["gaps"],
            "metrics": zenith["metrics"],
        },
        "path_relationships": dict(sorted(relations.items())),
        "related_zenith_eligibility": dict(sorted(eligibility.items())),
        "buckets": {key: dict(value) for key, value in sorted(buckets.items())},
        "limitations": [
            "Mole's displayed sizes are rounded potential bytes, not verified reclaim.",
            "Nested preview paths may be counted more than once in its displayed sum.",
            "Path ancestry does not prove equal cleanup units or equal authorization.",
            "du measures current allocated footprint, not the bytes an owner command would remove.",
            "Neither input proves actual deletion throughput or disk free-space change.",
        ],
    }
    if args.private_details:
        summary["private_details"] = details
    print(json.dumps(summary, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
