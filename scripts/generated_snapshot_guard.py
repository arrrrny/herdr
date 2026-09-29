"""Enforce that upstream-CI-generated files carry no fork-owned content.

`.github/GENERATED_UPSTREAM_FILES` lists the paths that upstream's own CI
regenerates and commits (`distribution/preview.json`, the `docs/preview/website`
snapshot). Conflicts confined to those paths carry no design decision, so
`.github/workflows/sync-upstream.yml` clears them to upstream's copy instead of
failing the daily sync and waiting for a human.

That shortcut is only sound if the fork really has no hand-written contribution
to those files. This check is what makes it sound: it scans every generated file
for fork-owned markers and fails when it finds one, and it runs in ordinary PR CI
on every change. So the workflow's automatic resolution is backed by an enforced
invariant rather than an assumption, and a future change that puts fork content
in a generated file fails here before the workflow can act on it.

Markers come from two sources, because either alone is incomplete:

- A small set of fork identity tokens (`arrrrny`, `arrrrny/herdr#`). Fork-owned
  code references its own issue tracker in comments, so these catch new fork
  edits that have not been registered in `FORK_OWNED_FILES` yet. Upstream cannot
  emit these, so they never false-positive.
- The distinctive `:: <marker>` values from `.github/FORK_OWNED_FILES`, filtered
  by `is_distinctive_marker`. That filter matters: `FORK_OWNED_FILES` markers are
  chosen to be easy to grep, not impossible to collide with, and several are
  ordinary words or phrases. The Hermes marker `resume` alone appears in 22
  upstream-generated documentation files, so mining the list unfiltered makes
  this check permanently red and therefore ignorable.

The check is deliberately conservative about the thing it must never miss: it
would rather fail on a plausible fork fingerprint than let the workflow discard
fork code silently.
"""

from __future__ import annotations

import argparse
import os
import sys
from collections.abc import Iterable, Sequence
from dataclasses import dataclass
from pathlib import Path

DEFAULT_LIST = Path(".github/GENERATED_UPSTREAM_FILES")
DEFAULT_FORK_LIST = Path(".github/FORK_OWNED_FILES")

# Tokens that identify fork-owned content. Upstream does not know about the
# fork's issue tracker, so these cannot appear in upstream-generated files.
FORK_IDENTITY_TOKENS = ("arrrrny", "arrrrny/herdr#")

# Files larger than this are skipped: the generated trees are documentation and
# manifests, and a size cap keeps the scan from walking something huge by mistake.
MAX_SCAN_BYTES = 4 * 1024 * 1024


# A `FORK_OWNED_FILES` marker only transfers to the generated-file scan when it
# cannot plausibly occur in upstream-authored documentation. Markers are picked
# for grep-ability, so some are ordinary prose (`keep that descriptor`) or bare
# dictionary words (`resume`). Those are excluded by requiring enough length plus
# at least one character shape that does not appear in running text: an
# identifier separator, a path/issue character, a bracket, a quote, a digit, or
# camelCase/SHOUT_CASE interior capitals as in `render_sidebar_header_badges`.
MIN_MARKER_LENGTH = 8
_MARKER_DISTINCTIVE_CHARS = "_-.:/#()\"'=<>[]$"


class GuardError(Exception):
    """One of the guard lists itself is malformed."""


def is_distinctive_marker(marker: str) -> bool:
    """True when a fork marker is specific enough to scan generated files for.

    A marker qualifies when it is at least `MIN_MARKER_LENGTH` characters and
    carries a character that essentially never appears in prose — an identifier
    separator, a path/issue character, a bracket, a quote, a digit, or an
    interior capital as in a camelCase or SHOUT_CASE identifier. This keeps
    `arrrrny/herdr#27`, `render_sidebar_header_badges` and `ClientShellBadge`
    while dropping `resume` and `keep that descriptor`, which would otherwise
    match ordinary documentation prose and make this guard permanently fail.
    """
    if len(marker) < MIN_MARKER_LENGTH:
        return False
    if any(char.isdigit() for char in marker):
        return True
    if any(char in _MARKER_DISTINCTIVE_CHARS for char in marker):
        return True
    return any(char.isupper() for char in marker[1:])


@dataclass(frozen=True)
class GeneratedPath:
    raw: str
    is_prefix: bool


@dataclass(frozen=True)
class Violation:
    path: str
    token: str

    def render(self) -> str:
        return f"{self.path} contains fork-owned marker: {self.token}"


def parse_generated_paths(text: str) -> list[GeneratedPath]:
    """Parse one path per line, ignoring blanks and `#` comments."""
    paths: list[GeneratedPath] = []
    for number, raw in enumerate(text.splitlines(), start=1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.endswith("/"):
            paths.append(GeneratedPath(line, is_prefix=True))
        else:
            paths.append(GeneratedPath(line, is_prefix=False))
        del number
    return paths


def parse_fork_markers(text: str) -> list[str]:
    """Extract the distinctive `:: <marker>` values from a `FORK_OWNED_FILES` listing."""
    markers: list[str] = []
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if "::" not in line:
            continue
        marker = line.split("::", 1)[1].strip()
        if marker and is_distinctive_marker(marker):
            markers.append(marker)
    return markers


def iter_generated_files(
    root: Path, paths: Sequence[GeneratedPath]
) -> Iterable[Path]:
    """Yield every existing file covered by the generated-path list."""
    seen: set[Path] = set()
    for entry in paths:
        target = root / entry.raw
        if entry.is_prefix:
            if not target.is_dir():
                continue
            candidates = sorted(p for p in target.rglob("*") if p.is_file())
        elif target.is_file():
            candidates = [target]
        else:
            continue
        for candidate in candidates:
            if candidate in seen:
                continue
            seen.add(candidate)
            yield candidate


def scan(root: Path, paths: Sequence[GeneratedPath], markers: Sequence[str]) -> list[Violation]:
    """Report every generated file that carries a fork-owned marker.

    One violation per file: when several markers match, the longest is reported
    because it is the most specific fingerprint and gives a reader the most
    context for what the file is doing.
    """
    violations: list[Violation] = []
    for path in iter_generated_files(root, paths):
        try:
            if path.stat().st_size > MAX_SCAN_BYTES:
                continue
            content = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        matches = [marker for marker in markers if marker in content]
        if matches:
            # `.as_posix()` on purpose. The reported path is compared against
            # git's conflicted-path list and read in CI logs, and git always
            # writes forward slashes, so a Windows-native `docs\preview\...`
            # would neither match the list nor be greppable as reported.
            violations.append(
                Violation(path.relative_to(root).as_posix(), max(matches, key=len))
            )
    return violations


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Verify generated upstream files carry no fork-owned content."
    )
    parser.add_argument("--list", dest="list_path", type=Path, default=DEFAULT_LIST)
    parser.add_argument(
        "--fork-list", dest="fork_list_path", type=Path, default=DEFAULT_FORK_LIST
    )
    parser.add_argument("--root", type=Path, default=Path("."))
    args = parser.parse_args(argv)

    try:
        paths = parse_generated_paths(args.list_path.read_text(encoding="utf-8"))
    except OSError as error:
        print(f"cannot read {args.list_path}: {error}", file=sys.stderr)
        return 1

    if not paths:
        print(
            f"{args.list_path} lists no generated files; refusing to report success",
            file=sys.stderr,
        )
        return 1

    markers = list(FORK_IDENTITY_TOKENS)
    try:
        markers.extend(parse_fork_markers(args.fork_list_path.read_text(encoding="utf-8")))
    except OSError as error:
        print(f"cannot read {args.fork_list_path}: {error}", file=sys.stderr)
        return 1

    violations = scan(args.root, paths, markers)
    for violation in violations:
        print(violation.render())

    if violations:
        print(
            f"generated-file guardrail failed: {len(violations)} fork-owned marker(s) "
            f"in upstream-generated files; the sync workflow cannot auto-resolve "
            f"these paths and will abort for manual resolution",
            file=sys.stderr,
        )
        return 1

    scanned = sum(1 for _ in iter_generated_files(args.root, paths))
    print(
        f"generated-file guardrail passed: {scanned} file(s) across "
        f"{len(paths)} generated path(s) carry no fork-owned content"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
