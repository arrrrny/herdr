"""Pin the generated-conflict resolver that the daily sync depends on.

`scripts/sync_resolve_generated.py` is the one place where the fork lets a
conflicted path resolve to upstream without a human. A bug here is silent data
loss in the direction the fork sync policy exists to prevent, so the tests cover
the refusal paths just as hard as the happy path: a single real conflict must
send the whole merge back to manual resolution, and a failed invariant check must
stop before anything is written.
"""

from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path

from scripts.generated_snapshot_guard import parse_generated_paths
from scripts.sync_resolve_generated import Decision, classify, main as resolve_main

LIST = "distribution/preview.json\ndocs/preview/\n"


class ClassifyTests(unittest.TestCase):
    def setUp(self) -> None:
        self.paths = parse_generated_paths(LIST)

    def test_literal_entry_matches_only_itself(self) -> None:
        decision = classify(["distribution/preview.json"], self.paths)
        self.assertEqual(decision, Decision(("distribution/preview.json",), ()))
        self.assertTrue(decision.all_generated)

    def test_prefix_entry_matches_everything_underneath(self) -> None:
        conflicted = [
            "docs/preview/website/src/content/docs/agents.mdx",
            "docs/preview/website/src/data/config-reference.json",
        ]
        decision = classify(conflicted, self.paths)
        self.assertEqual(decision.real, ())
        self.assertEqual(len(decision.generated), 2)

    def test_prefix_entry_does_not_match_a_sibling_directory(self) -> None:
        # `docs/preview/website` must not swallow `docs/preview-notes/...`.
        decision = classify(["docs/preview-notes/agents.mdx"], self.paths)
        self.assertEqual(decision.generated, ())
        self.assertEqual(decision.real, ("docs/preview-notes/agents.mdx",))

    def test_real_code_conflict_is_not_generated(self) -> None:
        decision = classify(["src/pane.rs"], self.paths)
        self.assertEqual(decision.generated, ())
        self.assertEqual(decision.real, ("src/pane.rs",))

    def test_mixed_conflicts_are_refused_whole(self) -> None:
        decision = classify(
            ["docs/preview/website/agents.mdx", "src/pane.rs"], self.paths
        )
        self.assertEqual(decision.generated, ("docs/preview/website/agents.mdx",))
        self.assertEqual(decision.real, ("src/pane.rs",))
        self.assertFalse(decision.all_generated)

    def test_empty_conflict_set_is_not_all_generated(self) -> None:
        self.assertFalse(classify([], self.paths).all_generated)

    def test_blank_entries_are_ignored(self) -> None:
        self.assertEqual(classify(["", "src/pane.rs"], self.paths).real, ("src/pane.rs",))

    def test_results_are_sorted_for_stable_reporting(self) -> None:
        decision = classify(["src/b.rs", "src/a.rs", "docs/preview/z.mdx"], self.paths)
        self.assertEqual(decision.generated, ("docs/preview/z.mdx",))
        self.assertEqual(decision.real, ("src/a.rs", "src/b.rs"))


def _git_repo(root: Path) -> None:
    subprocess.run(["git", "init", "--quiet", "-b", "main", str(root)], check=True)
    for name, value in (("user.email", "t@example.invalid"), ("user.name", "T")):
        subprocess.run(["git", "-C", str(root), "config", name, value], check=True)


def _write(root: Path, relative: str, content: str) -> None:
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


class EndToEndTests(unittest.TestCase):
    """Drive the resolver against real conflicted git merges."""

    def _base_repo(self, root: Path) -> None:
        _git_repo(root)
        _write(root, "src/pane.rs", "base\n")
        _write(root, "docs/preview/website/agents.mdx", "base docs\n")
        _write(root, "distribution/preview.json", '{"base": true}\n')
        subprocess.run(["git", "-C", str(root), "add", "-A"], check=True)
        subprocess.run(
            ["git", "-C", str(root), "commit", "--quiet", "-m", "base"], check=True
        )

    def test_real_conflict_aborts_and_writes_nothing(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._base_repo(root)
            # Build the exact topology the sync produces: master and an
            # upstream branch each editing the same files.
            subprocess.run(["git", "-C", str(root), "checkout", "--quiet", "-b", "upstream-side"], check=True)
            _write(root, "docs/preview/website/agents.mdx", "upstream docs\n")
            _write(root, "src/pane.rs", "upstream pane\n")
            subprocess.run(["git", "-C", str(root), "commit", "--quiet", "-am", "upstream"], check=True)
            subprocess.run(["git", "-C", str(root), "checkout", "--quiet", "main"], check=True)
            _write(root, "docs/preview/website/agents.mdx", "fork docs\n")
            _write(root, "src/pane.rs", "fork pane\n")
            subprocess.run(["git", "-C", str(root), "commit", "--quiet", "-am", "fork"], check=True)

            merge = subprocess.run(
                ["git", "-C", str(root), "merge", "upstream-side", "--no-edit"],
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(merge.returncode, 0, "expected a conflicted merge")

            list_path = root / ".github/GENERATED_UPSTREAM_FILES"
            list_path.parent.mkdir(parents=True, exist_ok=True)
            list_path.write_text(LIST, encoding="utf-8")
            fork_list = root / ".github/FORK_OWNED_FILES"
            fork_list.write_text("src/pane.rs :: arrrrny/herdr#1\n", encoding="utf-8")

            code = resolve_main(["--list", str(list_path), "--root", str(root)])
            self.assertEqual(code, 1, "a real conflict must abort")
            # The merge is untouched: still conflicted, nothing committed.
            unmerged = subprocess.run(
                ["git", "-C", str(root), "diff", "--name-only", "--diff-filter=U"],
                capture_output=True,
                text=True,
                check=True,
            ).stdout.split()
            self.assertIn("src/pane.rs", unmerged)
            self.assertIn("docs/preview/website/agents.mdx", unmerged)

    def test_generated_only_conflict_resolves_to_upstream(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            _git_repo(root)
            _write(root, "docs/preview/website/agents.mdx", "base docs\n")
            _write(root, "src/pane.rs", "base pane\n")
            subprocess.run(["git", "-C", str(root), "add", "-A"], check=True)
            subprocess.run(["git", "-C", str(root), "commit", "--quiet", "-m", "base"], check=True)
            subprocess.run(["git", "-C", str(root), "checkout", "--quiet", "-b", "upstream-side"], check=True)
            _write(root, "docs/preview/website/agents.mdx", "upstream docs\n")
            subprocess.run(["git", "-C", str(root), "commit", "--quiet", "-am", "upstream"], check=True)
            subprocess.run(["git", "-C", str(root), "checkout", "--quiet", "main"], check=True)
            _write(root, "docs/preview/website/agents.mdx", "fork docs\n")
            subprocess.run(["git", "-C", str(root), "commit", "--quiet", "-am", "fork"], check=True)
            merge = subprocess.run(
                ["git", "-C", str(root), "merge", "upstream-side", "--no-edit"],
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(merge.returncode, 0, "expected a conflicted merge")

            list_path = root / ".github/GENERATED_UPSTREAM_FILES"
            list_path.parent.mkdir(parents=True, exist_ok=True)
            list_path.write_text(LIST, encoding="utf-8")
            fork_list = root / ".github/FORK_OWNED_FILES"
            fork_list.write_text("src/pane.rs :: arrrrny/herdr#1\n", encoding="utf-8")
            code = resolve_main(["--list", str(list_path), "--root", str(root)])
            self.assertEqual(code, 0)
            content = (root / "docs/preview/website/agents.mdx").read_text(encoding="utf-8")
            self.assertEqual(content, "upstream docs\n")
            unmerged = subprocess.run(
                ["git", "-C", str(root), "diff", "--name-only", "--diff-filter=U"],
                capture_output=True,
                text=True,
                check=True,
            ).stdout.strip()
            self.assertEqual(unmerged, "")

    def test_fork_marker_in_generated_file_blocks_resolution(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            _git_repo(root)
            _write(root, "docs/preview/website/agents.mdx", "base docs\n")
            subprocess.run(["git", "-C", str(root), "add", "-A"], check=True)
            subprocess.run(["git", "-C", str(root), "commit", "--quiet", "-m", "base"], check=True)
            subprocess.run(["git", "-C", str(root), "checkout", "--quiet", "-b", "upstream-side"], check=True)
            _write(root, "docs/preview/website/agents.mdx", "upstream arrrrny/herdr#9 docs\n")
            subprocess.run(["git", "-C", str(root), "commit", "--quiet", "-am", "upstream"], check=True)
            subprocess.run(["git", "-C", str(root), "checkout", "--quiet", "main"], check=True)
            _write(root, "docs/preview/website/agents.mdx", "fork docs\n")
            subprocess.run(["git", "-C", str(root), "commit", "--quiet", "-am", "fork"], check=True)
            subprocess.run(
                ["git", "-C", str(root), "merge", "upstream-side", "--no-edit"],
                capture_output=True,
                text=True,
            )

            list_path = root / ".github/GENERATED_UPSTREAM_FILES"
            list_path.parent.mkdir(parents=True, exist_ok=True)
            list_path.write_text(LIST, encoding="utf-8")
            fork_list = root / ".github/FORK_OWNED_FILES"
            fork_list.write_text("src/pane.rs :: arrrrny/herdr#1\n", encoding="utf-8")
            code = resolve_main(["--list", str(list_path), "--root", str(root)])
            self.assertEqual(code, 1, "a fork marker must block automatic resolution")
            # Blocked means untouched: the path is still an open conflict.
            unmerged = subprocess.run(
                ["git", "-C", str(root), "diff", "--name-only", "--diff-filter=U"],
                capture_output=True,
                text=True,
                check=True,
            ).stdout.split()
            self.assertIn("docs/preview/website/agents.mdx", unmerged)


if __name__ == "__main__":
    unittest.main()
