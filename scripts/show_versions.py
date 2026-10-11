#!/usr/bin/env python3
"""Prints the version of each publishable package.

Each package is released independently, so it is easy to bump two and forget
the third. Used by CI to report, by bump_versions.py to check, and by the
release workflow to compare against the tag.

With --quiet, prints only the highest version and nothing else, for use in a
shell substitution.
"""

import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent

# Logical package name -> manifest path.
MANIFESTS = {
    "rust": "rust/Cargo.toml",
    "python": "python/pyproject.toml",
    "javascript": "javascript/package.json",
}


def read_versions() -> dict[str, str | None]:
    versions: dict[str, str | None] = {}
    for name, relative in MANIFESTS.items():
        text = (ROOT / relative).read_text()
        if name == "javascript":
            versions[name] = json.loads(text).get("version")
        else:
            match = re.search(r'^version\s*=\s*"([^"]+)"', text, re.MULTILINE)
            versions[name] = match.group(1) if match else None
    return versions


def _semver_key(version: str):
    """Sort key for semver-ish versions, ignoring any pre-release suffix."""
    match = re.match(r"(\d+)\.(\d+)\.(\d+)", version or "")
    if not match:
        return (0, 0, 0)
    return tuple(int(part) for part in match.groups())


def main() -> int:
    quiet = "--quiet" in sys.argv[1:]
    versions = read_versions()
    missing = [name for name, version in versions.items() if not version]

    if quiet:
        if missing:
            print(f"", file=sys.stderr)
            for name in missing:
                print(f"could not read a version for {name}", file=sys.stderr)
            raise SystemExit(1)
        # The release workflow compares this against the git tag. Use the
        # highest so a lagging package cannot make a valid tag look wrong.
        print(max(versions.values(), key=_semver_key))
        return 0

    for name, version in versions.items():
        print(f"  {name:<12} {version or '(not found)'}")

    if missing:
        print(f"\nCould not read a version for: {', '.join(missing)}", file=sys.stderr)
        return 1

    # Not an error: the packages have independent release histories.
    unique = set(versions.values())
    if len(unique) > 1:
        print(f"\n  note: versions differ across packages ({', '.join(sorted(unique))}).")
        print("        This is allowed; just confirm it is intentional.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
