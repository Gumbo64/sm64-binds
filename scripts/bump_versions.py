#!/usr/bin/env python3
"""Sets the version of every publishable package in one go.

Each package carries its version in a different manifest and each registry has
its own numbering, so bumping all three is a fiddly, easy-to-miss chore. This
edits them together and, with --sync, copies the version of one package to the
others.

    bump_versions.py 1.0.25           # set every package to 1.0.25
    bump_versions.py --from rust      # copy rust's version to the others
    bump_versions.py --sync           # alias for --from rust

Prints the resulting versions; makes no commit.
"""

import argparse
import json
import pathlib
import re
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from show_versions import MANIFESTS, ROOT, read_versions  # noqa: E402

SEMVER = re.compile(
    r"^(\d+)\.(\d+)\.(\d+)"
    r"(?:-([0-9A-Za-z.-]+))?"
    r"(?:\+([0-9A-Za-z.-]+))?$"
)


def set_version(name: str, version: str) -> None:
    path = ROOT / MANIFESTS[name]
    text = path.read_text()

    if name == "javascript":
        # package.json is JSON; parse and re-dump to avoid mangling anything.
        data = json.loads(text)
        data["version"] = version
        path.write_text(json.dumps(data, indent=2) + "\n")
        return

    # Only the [package] version, not a dependency's.
    updated, count = re.subn(
        r'^version\s*=\s*"[^"]+"',
        f'version = "{version}"',
        text,
        count=1,
        flags=re.MULTILINE,
    )
    if count != 1:
        raise SystemExit(f"{name}: could not find a version line in {MANIFESTS[name]}")
    path.write_text(updated)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("version", nargs="?", help="version to set everywhere, e.g. 1.0.25")
    group.add_argument("--from", dest="source", metavar="PACKAGE",
                       choices=sorted(MANIFESTS), help="copy this package's version to the others")
    group.add_argument("--sync", action="store_true",
                       help="alias for --from rust")
    args = parser.parse_args()

    if args.version:
        if not SEMVER.match(args.version):
            raise SystemExit(f"'{args.version}' is not a semver version")
        target = args.version
    else:
        source = "rust" if args.sync else args.source
        current = read_versions().get(source)
        if not current:
            raise SystemExit(f"could not read the current {source} version")
        target = current
        print(f"Copying {source}'s version ({target}) to the other packages.")

    print(f"\nSetting every package to {target}:")
    for name in sorted(MANIFESTS):
        set_version(name, target)
        print(f"  {name}")

    print()
    for name, version in read_versions().items():
        print(f"  {name:<12} {version}")

    print("\nReview the diff, then commit and tag:")
    print(f"  git commit -am 'Release {target}'")
    print(f"  git tag v{target} && git push --tags")
    return 0


if __name__ == "__main__":
    sys.exit(main())
