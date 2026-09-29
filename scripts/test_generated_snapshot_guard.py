"""Pin the generated-file guardrail that licenses automatic sync resolution.

`.github/workflows/sync-upstream.yml` clears merge conflicts confined to
`.github/GENERATED_UPSTREAM_FILES` without human review. The only reason that is
safe instead of a silent data-loss bug is `scripts/generated_snapshot_guard.py`,
which proves those files carry no fork-owned content on every CI run.

That makes this guard's failure modes unusually expensive:

- A false positive makes the check permanently red, so the daily sync keeps
  aborting for manual resolution and nobody reads the signal anymore. This is
  what happened while building it: the bare `resume` marker from the Hermes
  entry appears in 22 upstream-generated documentation files.
- A false negative lets the workflow resolve a conflict to upstream's copy while
  a fork edit sits in it, which is exactly the loss the fork sync policy exists
  to prevent.

So the tests below pin the marker filter (the false-positive guard), the path
parsing, the violation report, and the refusal to succeed on an empty list.
"""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from scripts.generated_snapshot_guard import (
    FORK_IDENTITY_TOKENS,
    is_distinctive_marker,
    parse_fork_markers,
    parse_generated_paths,
    scan,
)
from scripts.generated_snapshot_guard import main as guard_main


class GeneratedPathParsingTests(unittest.TestCase):
    def test_trailing_slash_marks_a_directory_prefix(self) -> None:
        paths = parse_generated_paths("distribution/preview.json\ndocs/preview/\n")
        self.assertEqual([p.raw for p in paths], ["distribution/preview.json", "docs/preview/"])
        self.assertEqual([p.is_prefix for p in paths], [False, True])

    def test_comments_and_blank_lines_are_ignored(self) -> None:
        text = "# a comment\n\n  \ndocs/preview/\n"
        self.assertEqual(len(parse_generated_paths(text)), 1)

    def test_indented_entries_are_trimmed(self) -> None:
        paths = parse_generated_paths("   docs/preview/   \n")
        self.assertEqual(paths[0].raw, "docs/preview/")


class DistinctiveMarkerTests(unittest.TestCase):
    def test_identifiers_paths_and_issue_refs_are_distinctive(self) -> None:
        for marker in (
            "arrrrny/herdr#27",
            "render_sidebar_header_badges",
            "ClientShellBadge",
            "ClearIfNonKey",
            "$document.geometries",
            "merge_badges",
            "PTY_WRITE_CHUNK_BYTES",
        ):
            with self.subTest(marker=marker):
                self.assertTrue(is_distinctive_marker(marker))

    def test_prose_and_dictionary_words_are_not_distinctive(self) -> None:
        # `resume` is a real FORK_OWNED_FILES marker and appears in 22 upstream
        # generated documentation files. Admitting it would make this guard
        # permanently red and therefore ignorable.
        for marker in (
            "resume",
            "keep that descriptor",
            "mod badge",
            "current",
        ):
            with self.subTest(marker=marker):
                self.assertFalse(is_distinctive_marker(marker))

    def test_short_markers_are_rejected_even_with_separators(self) -> None:
        self.assertFalse(is_distinctive_marker("a_b_c"))
        self.assertFalse(is_distinctive_marker("x.y.z"))

    def test_marker_needs_interior_capital_not_just_a_leading_one(self) -> None:
        self.assertFalse(is_distinctive_marker("Sentence case marker"))
        self.assertTrue(is_distinctive_marker("SentenceCaseMarker"))


class ForkMarkerParsingTests(unittest.TestCase):
    def test_only_distinctive_markers_are_harvested(self) -> None:
        text = (
            "# comment\n"
            "src/a.rs :: arrrrny/herdr#1\n"
            "src/b.rs :: resume\n"
            "src/c.rs :: render_sidebar_header_badges\n"
            "src/d.rs :: keep that descriptor\n"
        )
        self.assertEqual(
            parse_fork_markers(text),
            ["arrrrny/herdr#1", "render_sidebar_header_badges"],
        )

    def test_lines_without_a_separator_are_skipped(self) -> None:
        self.assertEqual(parse_fork_markers("not an entry\n"), [])


class ScanTests(unittest.TestCase):
    def _repo(self, files: dict[str, str]) -> Path:
        root = Path(tempfile.mkdtemp())
        for relative, content in files.items():
            path = root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")
        return root

    def test_clean_generated_tree_reports_no_violation(self) -> None:
        root = self._repo(
            {
                "distribution/preview.json": '{"build_id": "2026-09-28"}',
                "docs/preview/website/index.mdx": "Herdr is a terminal runtime.",
            }
        )
        paths = parse_generated_paths("distribution/preview.json\ndocs/preview/\n")
        self.assertEqual(scan(root, paths, FORK_IDENTITY_TOKENS), [])

    def test_fork_identity_token_in_a_generated_file_is_reported(self) -> None:
        root = self._repo(
            {"docs/preview/website/agents.mdx": "See arrrrny/herdr#1 for details."}
        )
        paths = parse_generated_paths("docs/preview/\n")
        violations = scan(root, paths, FORK_IDENTITY_TOKENS)
        # One violation per file, naming the most specific marker that matched.
        self.assertEqual(len(violations), 1)
        self.assertEqual(violations[0].token, "arrrrny/herdr#")

    def test_fork_marker_in_a_generated_file_is_reported_with_its_relative_path(self) -> None:
        root = self._repo(
            {"docs/preview/website/ja/agents.mdx": "x render_sidebar_header_badges y"}
        )
        paths = parse_generated_paths("docs/preview/\n")
        violations = scan(root, paths, ["render_sidebar_header_badges"])
        self.assertEqual(
            violations[0].path, "docs/preview/website/ja/agents.mdx"
        )

    def test_exact_path_entries_do_not_match_sibling_files(self) -> None:
        root = self._repo(
            {
                "distribution/preview.json": "arrrrny",
                "distribution/latest.json": "arrrrny",
            }
        )
        paths = parse_generated_paths("distribution/preview.json\n")
        violations = scan(root, paths, FORK_IDENTITY_TOKENS)
        self.assertEqual([v.path for v in violations], ["distribution/preview.json"])

    def test_missing_generated_paths_are_not_violations(self) -> None:
        root = self._repo({"README.md": "arrrrny"})
        paths = parse_generated_paths("docs/preview/\ndistribution/preview.json\n")
        self.assertEqual(scan(root, paths, FORK_IDENTITY_TOKENS), [])


class MainTests(unittest.TestCase):
    def test_empty_list_refuses_to_report_success(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "list").write_text("# only comments\n", encoding="utf-8")
            (root / "fork_list").write_text("src/a.rs :: arrrrny/herdr#1\n", encoding="utf-8")
            code = guard_main(
                [
                    "--list",
                    str(root / "list"),
                    "--fork-list",
                    str(root / "fork_list"),
                    "--root",
                    str(root),
                ]
            )
            self.assertEqual(code, 1)

    def test_violation_exits_nonzero(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "docs/preview").mkdir(parents=True)
            (root / "docs/preview/a.mdx").write_text("arrrrny/herdr#27", encoding="utf-8")
            (root / "list").write_text("docs/preview/\n", encoding="utf-8")
            (root / "fork_list").write_text("src/a.rs :: arrrrny/herdr#1\n", encoding="utf-8")
            code = guard_main(
                [
                    "--list",
                    str(root / "list"),
                    "--fork-list",
                    str(root / "fork_list"),
                    "--root",
                    str(root),
                ]
            )
            self.assertEqual(code, 1)

    def test_clean_tree_exits_zero(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "docs/preview").mkdir(parents=True)
            (root / "docs/preview/a.mdx").write_text("all clear", encoding="utf-8")
            (root / "list").write_text("docs/preview/\n", encoding="utf-8")
            (root / "fork_list").write_text("src/a.rs :: arrrrny/herdr#1\n", encoding="utf-8")
            code = guard_main(
                [
                    "--list",
                    str(root / "list"),
                    "--fork-list",
                    str(root / "fork_list"),
                    "--root",
                    str(root),
                ]
            )
            self.assertEqual(code, 0)

    def test_repository_lists_are_usable_and_consistent(self) -> None:
        """The checked-in lists must parse and agree with the real repository."""
        repo_root = Path(__file__).resolve().parent.parent
        list_path = repo_root / ".github/GENERATED_UPSTREAM_FILES"
        fork_list = repo_root / ".github/FORK_OWNED_FILES"
        if not list_path.is_file():
            self.skipTest("generated-file list is not present in this checkout")
        self.assertGreaterEqual(len(parse_generated_paths(list_path.read_text())), 1)
        self.assertGreaterEqual(len(parse_fork_markers(fork_list.read_text())), 10)


if __name__ == "__main__":
    unittest.main()
