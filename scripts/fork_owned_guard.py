"""Verify that fork-owned files still carry their survival markers.

The daily upstream sync merges upstream into this fork. A clean merge can still
drop fork code, so `.github/FORK_OWNED_FILES` lists `<path>  ::  <marker>`
entries that must survive it. This check runs only after a clean merge and fails
when an entry's file is gone, when the marker is no longer in it, or when the
list parses to nothing at all.

The parse lives here rather than inline in the workflow because the inline
`sed`-based version stripped only `:: <marker>` and kept the column padding on
the path, so every entry resolved to a missing file. A guardrail that reports
every entry as broken is indistinguishable from one that never reports at all,
so `scripts/test_fork_owned_guard.py` pins the parse, the report, and the exit
codes.

The workflow consumes `missing_files` and `marker_missing` from `$GITHUB_OUTPUT`
when that variable is set; stdout carries the human report. The step's own exit
status is the pass/fail signal.
"""

from __future__ import annotations

import argparse
import os
import sys
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path

DEFAULT_LIST = Path(".github/FORK_OWNED_FILES")


class GuardError(Exception):
    """The fork-owned list itself is malformed."""


@dataclass(frozen=True)
class Entry:
    path: str
    marker: str


@dataclass(frozen=True)
class Report:
    missing_files: tuple[str, ...]
    missing_markers: tuple[str, ...]

    @property
    def ok(self) -> bool:
        return not self.missing_files and not self.missing_markers


def parse_entries(text: str) -> list[Entry]:
    """Parse `<path>  ::  <marker>` lines, ignoring blanks and `#` comments."""
    entries: list[Entry] = []
    for number, raw in enumerate(text.splitlines(), start=1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if "::" not in line:
            raise GuardError(f"line {number}: missing '::' separator: {line!r}")
        path, marker = line.split("::", 1)
        path, marker = path.strip(), marker.strip()
        if not path or not marker:
            raise GuardError(f"line {number}: empty path or marker: {line!r}")
        entries.append(Entry(path=path, marker=marker))
    return entries


def check(root: Path, entries: Sequence[Entry]) -> Report:
    missing_files: list[str] = []
    missing_markers: list[str] = []
    for entry in entries:
        try:
            content = (root / entry.path).read_text(encoding="utf-8", errors="replace")
        except OSError:
            missing_files.append(entry.path)
            continue
        if entry.marker not in content:
            missing_markers.append(f"{entry.path} (missing marker: {entry.marker})")
    return Report(tuple(missing_files), tuple(missing_markers))


def write_github_output(handle: Path, report: Report) -> None:
    with handle.open("a", encoding="utf-8") as file:
        file.write("missing_files<<GUARD_EOF\n")
        file.write("".join(f"{path}\n" for path in report.missing_files))
        file.write("GUARD_EOF\n")
        file.write("marker_missing<<GUARD_EOF\n")
        file.write("".join(f"{entry}\n" for entry in report.missing_markers))
        file.write("GUARD_EOF\n")


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Verify fork-owned survival markers.")
    parser.add_argument("--list", dest="list_path", type=Path, default=DEFAULT_LIST)
    parser.add_argument("--root", type=Path, default=Path("."))
    parser.add_argument("--github-output", dest="github_output", type=Path, default=None)
    args = parser.parse_args(argv)

    try:
        entries = parse_entries(args.list_path.read_text(encoding="utf-8"))
    except OSError as error:
        print(f"cannot read {args.list_path}: {error}", file=sys.stderr)
        return 1
    except GuardError as error:
        print(f"{args.list_path}: {error}", file=sys.stderr)
        return 1

    if not entries:
        print(
            f"{args.list_path} lists no fork-owned files; refusing to report success",
            file=sys.stderr,
        )
        return 1

    report = check(args.root, entries)

    output_path = args.github_output
    if output_path is None and os.environ.get("GITHUB_OUTPUT"):
        output_path = Path(os.environ["GITHUB_OUTPUT"])
    if output_path is not None:
        write_github_output(output_path, report)

    for path in report.missing_files:
        print(f"missing file: {path}")
    for entry in report.missing_markers:
        print(f"missing marker: {entry}")

    if not report.ok:
        print(
            f"fork-owned guardrail failed: {len(report.missing_files)} missing file(s), "
            f"{len(report.missing_markers)} missing marker(s)",
            file=sys.stderr,
        )
        return 1

    print(f"fork-owned guardrail passed: {len(entries)} entries intact")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
