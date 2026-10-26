from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from scripts.conventional_commits import (
    commit_message_subject,
    exempt_subject,
    valid_subject,
)

SCRIPT = Path(__file__).resolve().parent / "conventional_commits.py"

# Default subjects produced by git tooling, never typed by an author (#83).
MERGE_FORMS = [
    "Merge branch 'feat/widget'",
    "Merge pull request #74 from arrrrny/feat/widget",
    "Merge remote-tracking branch 'origin/master' into sync/fork-sync-resolution-2026-10-15",
    "Merge commit 'abc123'",
    "Merge tag 'v1.2.3'",
]
REVERT_FORM = 'Revert "feat: add widget"'


def run_script(*argv: str, cwd: Path | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SCRIPT), *argv], capture_output=True, text=True, cwd=cwd
    )


class ExemptSubjectTests(unittest.TestCase):
    def test_git_generated_merge_and_revert_forms_are_exempt(self) -> None:
        for subject in [*MERGE_FORMS, REVERT_FORM]:
            with self.subTest(subject=subject):
                self.assertTrue(exempt_subject(subject))

    def test_author_typed_subjects_are_not_exempt(self) -> None:
        for subject in [
            "updated stuff",
            # git always capitalizes "Merge"; a hand-typed lowercase form is
            # not a machine-generated subject and stays invalid.
            "merge branch 'x'",
            # git revert always quotes the reverted subject.
            "Revert feat: add widget",
            # A conventional subject is never exempt.
            "fix: merge branch handling",
        ]:
            with self.subTest(subject=subject):
                self.assertFalse(exempt_subject(subject))


class ValidSubjectTests(unittest.TestCase):
    def test_conventional_subjects_are_valid(self) -> None:
        for subject in [
            "chore: initial",
            "fix(update): install selected channel",
            "feat!: breaking change",
        ]:
            with self.subTest(subject=subject):
                self.assertTrue(valid_subject(subject))

    def test_non_conventional_subjects_are_invalid(self) -> None:
        for subject in ["updated stuff", "Merge branch 'feat/widget'", ""]:
            with self.subTest(subject=subject):
                self.assertFalse(valid_subject(subject))


class SubjectArgumentTests(unittest.TestCase):
    def test_pr_merge_default_title_passes(self) -> None:
        result = run_script("Merge pull request #74 from arrrrny/feat/widget")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_git_revert_default_title_passes(self) -> None:
        result = run_script(REVERT_FORM)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_non_conventional_title_still_fails(self) -> None:
        result = run_script("updated stuff")
        self.assertEqual(result.returncode, 1)
        self.assertIn("updated stuff", result.stdout)


class MessageFileTests(unittest.TestCase):
    def test_message_file_with_merge_subject_passes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            message = Path(tmp) / "COMMIT_EDITMSG"
            message.write_text(
                "Merge remote-tracking branch 'origin/master' into branch\n",
                encoding="utf-8",
            )
            result = run_script("--message-file", str(message))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_message_file_first_subject_is_read(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            message = Path(tmp) / "COMMIT_EDITMSG"
            message.write_text(
                "fix: real subject\n# comment line\n", encoding="utf-8"
            )
            self.assertEqual(commit_message_subject(message), "fix: real subject")


class RangeValidationTests(unittest.TestCase):
    """End-to-end reproduction of the CI push job: --range before..after."""

    def setUp(self) -> None:
        self.repo = Path(tempfile.mkdtemp(prefix="issue83-range-"))
        self._git("init", "-q", "-b", "master", ".")
        self._git("config", "user.email", "t@example.test")
        self._git("config", "user.name", "t")

    def tearDown(self) -> None:
        shutil.rmtree(self.repo, ignore_errors=True)

    def _git(self, *argv: str) -> str:
        result = subprocess.run(
            ["git", *argv], cwd=self.repo, check=True, capture_output=True, text=True
        )
        return result.stdout.strip()

    def _commit_file(self, name: str, message: str) -> None:
        (self.repo / name).write_text(name, encoding="utf-8")
        self._git("add", name)
        self._git("commit", "-q", "-m", message)

    def test_merge_commit_subjects_pass_range_validation(self) -> None:
        self._commit_file("a.txt", "chore: initial")
        before = self._git("rev-parse", "HEAD")
        self._git("checkout", "-qb", "feat/widget")
        self._commit_file("b.txt", "feat: add widget")
        self._git("checkout", "-q", "master")
        self._git("merge", "--no-ff", "--no-edit", "-q", "feat/widget")
        self._git(
            "commit", "-q", "--allow-empty", "-m",
            "Merge pull request #74 from arrrrny/feat/widget",
        )
        self._git(
            "commit", "-q", "--allow-empty", "-m",
            "Merge remote-tracking branch 'origin/master' into sync/fork-sync-resolution-2026-10-15",
        )
        after = self._git("rev-parse", "HEAD")

        result = run_script("--range", f"{before}..{after}", cwd=self.repo)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_author_subject_still_fails_range_validation(self) -> None:
        self._commit_file("a.txt", "chore: initial")
        before = self._git("rev-parse", "HEAD")
        self._commit_file("b.txt", "updated the widget")
        after = self._git("rev-parse", "HEAD")

        result = run_script("--range", f"{before}..{after}", cwd=self.repo)
        self.assertEqual(result.returncode, 1)
        self.assertIn("updated the widget", result.stdout)


if __name__ == "__main__":
    unittest.main()
