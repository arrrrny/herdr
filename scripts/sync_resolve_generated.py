"""Clear upstream-sync conflicts that are confined to generated files.

The daily sync (`merge upstream/master` from `master`) must never resolve a real
conflict in favor of upstream: conflicts are where fork-owned features are
destroyed. But a large share of historical conflicts have been in the trees
upstream's own CI regenerates and commits, listed in
`.github/GENERATED_UPSTREAM_FILES` — 14 of the 19 in arrrrny/herdr#67 alone.
Neither side hand-authors those files, so the conflict carries no design
decision and upstream's freshly generated copy is the only correct resolution.
Left to a human they block the daily sync for a snapshot that Preview CI is
about to overwrite anyway.

This script makes that one narrow exception, and only when it cannot lose data:

- Every conflicted path must be a generated path. A single real conflict sends
  the whole merge down the untouched manual path, generated files included.
- `scripts/generated_snapshot_guard.py` must prove the generated files hold no
  fork-owned marker. It runs in ordinary PR CI, so "generated files never carry
  fork content" is an enforced invariant rather than an assumption here.

If either condition fails, nothing is written and the caller aborts the merge.
This is deliberately not a `-X theirs` merge strategy: the decision is per-path,
guarded, and refused as a whole the moment real code is involved.

The logic lives in a script rather than inline in the workflow because inline
`sed`-based parsing in `sync-upstream.yml` is what previously made
`fork_owned_guard.py` report every entry as broken. Workflow shell is not a
place where a subtle path-matching bug can be tested.
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path

# The workflow runs this file as `python3 scripts/sync_resolve_generated.py`,
# which puts `scripts/` on sys.path rather than the repository root, so the
# package-qualified import below would fail. Put the root first so the same
# import works both as a script (CI) and as `scripts.sync_resolve_generated`
# (the unit tests, which import it the way `python3 -m unittest` does).
_REPO_ROOT = Path(__file__).resolve().parent.parent
if str(_REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(_REPO_ROOT))

from scripts.generated_snapshot_guard import (  # noqa: E402
    DEFAULT_LIST,
    GuardError,
    parse_generated_paths,
)


@dataclass(frozen=True)
class Decision:
    generated: tuple[str, ...]
    real: tuple[str, ...]

    @property
    def all_generated(self) -> bool:
        return bool(self.generated) and not self.real


def classify(conflicted: Sequence[str], paths: Sequence) -> Decision:
    """Split conflicted paths into generated and real.

    `paths` is the parsed `.github/GENERATED_UPSTREAM_FILES` list. A path is
    generated when it equals a literal entry or falls under a trailing-slash
    directory prefix.
    """
    generated: list[str] = []
    real: list[str] = []
    for path in conflicted:
        if not path:
            continue
        if any(_matches(path, entry.raw, entry.is_prefix) for entry in paths):
            generated.append(path)
        else:
            real.append(path)
    return Decision(tuple(sorted(generated)), tuple(sorted(real)))


def _matches(path: str, raw: str, is_prefix: bool) -> bool:
    if is_prefix:
        return path.startswith(raw)
    return path == raw


def conflicted_paths(root: Path) -> list[str]:
    """Ask git which paths are currently unmerged."""
    result = subprocess.run(
        ["git", "diff", "--name-only", "--diff-filter=U"],
        cwd=root,
        capture_output=True,
        text=True,
        check=True,
    )
    return sorted(line for line in result.stdout.splitlines() if line.strip())


def resolve_generated(root: Path, paths: Sequence[str]) -> int:
    """Check out upstream's copy of `paths` and stage them.

    `git merge upstream/master` runs from `master`, so the fork is `--ours` and
    upstream is `--theirs`.
    """
    if paths:
        subprocess.run(
            ["git", "checkout", "--theirs", "--", *paths],
            cwd=root,
            check=True,
        )
        subprocess.run(["git", "add", "--", *paths], cwd=root, check=True)
    return 0


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Clear upstream-sync conflicts confined to upstream-generated files."
        )
    )
    parser.add_argument("--list", dest="list_path", type=Path, default=DEFAULT_LIST)
    parser.add_argument("--root", type=Path, default=Path("."))
    args = parser.parse_args(argv)
    root = args.root

    try:
        paths = parse_generated_paths(args.list_path.read_text(encoding="utf-8"))
    except (OSError, GuardError) as error:
        print(f"cannot read {args.list_path}: {error}", file=sys.stderr)
        return 1
    if not paths:
        print(f"{args.list_path} lists no generated files", file=sys.stderr)
        return 1

    conflicted = conflicted_paths(root)
    if not conflicted:
        print("no unmerged paths")
        return 0

    decision = classify(conflicted, paths)
    print(f"{len(decision.generated)} generated, {len(decision.real)} real conflict(s)")
    for path in decision.generated:
        print(f"  generated: {path}")
    for path in decision.real:
        print(f"  real:      {path}")

    if not decision.all_generated:
        print(
            "real conflicts present; leaving the merge for manual resolution",
            file=sys.stderr,
        )
        return 1

    # The invariant must hold before anything is written. The guard is located
    # relative to this script rather than the working directory so the resolver
    # cannot silently "pass" by finding nothing when invoked from elsewhere;
    # `check=False` plus an explicit return-code test keeps a missing or crashed
    # guard on the refusal side of the branch.
    guard = subprocess.run(
        [
            sys.executable,
            str(_REPO_ROOT / "scripts" / "generated_snapshot_guard.py"),
            "--root",
            str(root),
        ],
        cwd=root,
    )
    if guard.returncode != 0:
        print(
            "generated-file guardrail did not pass; leaving the merge for manual resolution",
            file=sys.stderr,
        )
        return 1

    resolve_generated(root, list(decision.generated))
    subprocess.run(
        ["git", "commit", "--no-edit"],
        cwd=root,
        check=True,
    )
    print(f"cleared {len(decision.generated)} generated conflict(s) to upstream")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
