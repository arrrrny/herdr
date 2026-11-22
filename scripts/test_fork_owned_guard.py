import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from scripts import fork_owned_guard as guard


def write_list(root: Path, text: str) -> Path:
    path = root / "FORK_OWNED_FILES"
    path.write_text(text, encoding="utf-8")
    return path


class ParseEntriesTests(unittest.TestCase):
    def test_parse_strips_column_padding(self) -> None:
        # Regression: the workflow's inline sed kept the padding on the path, so
        # every entry resolved to a missing file.
        entries = guard.parse_entries(
            "src/badges.rs                        ::  badge.set\n"
        )

        self.assertEqual(entries, [guard.Entry(path="src/badges.rs", marker="badge.set")])

    def test_parse_skips_comments_and_blank_lines(self) -> None:
        entries = guard.parse_entries(
            "# header\n"
            "\n"
            "src/pane.rs   ::  arrrrny/herdr#1\n"
            "   \n"
            "# trailing note\n"
        )

        self.assertEqual(entries, [guard.Entry(path="src/pane.rs", marker="arrrrny/herdr#1")])

    def test_parse_keeps_marker_separators_after_the_first(self) -> None:
        entries = guard.parse_entries("src/api/wait.rs  ::  a::b\n")

        self.assertEqual(entries, [guard.Entry(path="src/api/wait.rs", marker="a::b")])

    def test_parse_rejects_line_without_separator(self) -> None:
        with self.assertRaises(guard.GuardError):
            guard.parse_entries("src/badges.rs   badge.set\n")

    def test_parse_rejects_empty_marker(self) -> None:
        with self.assertRaises(guard.GuardError):
            guard.parse_entries("src/badges.rs   ::\n")


class CheckTests(unittest.TestCase):
    def test_check_reports_missing_file(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            report = guard.check(Path(raw), [guard.Entry("gone.rs", "marker")])

        self.assertEqual(report.missing_files, ("gone.rs",))
        self.assertEqual(report.missing_markers, ())
        self.assertFalse(report.ok)

    def test_check_reports_missing_marker(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            (root / "kept.rs").write_text("nothing here\n", encoding="utf-8")
            report = guard.check(root, [guard.Entry("kept.rs", "marker")])

        self.assertEqual(report.missing_files, ())
        self.assertEqual(report.missing_markers, ("kept.rs (missing marker: marker)",))
        self.assertFalse(report.ok)

    def test_check_passes_when_file_and_marker_are_present(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            (root / "kept.rs").write_text("// marker\n", encoding="utf-8")
            report = guard.check(root, [guard.Entry("kept.rs", "marker")])

        self.assertTrue(report.ok)


class MainTests(unittest.TestCase):
    def run_main(self, args: list[str]) -> int:
        with patch.dict(os.environ, {}, clear=False):
            os.environ.pop("GITHUB_OUTPUT", None)
            return guard.main(args)

    def test_main_passes_for_intact_entries(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            (root / "kept.rs").write_text("// marker\n", encoding="utf-8")
            listed = write_list(root, "kept.rs   ::  marker\n")

            self.assertEqual(self.run_main(["--list", str(listed), "--root", str(root)]), 0)

    def test_main_fails_and_reports_missing_marker(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            (root / "kept.rs").write_text("nothing here\n", encoding="utf-8")
            listed = write_list(root, "kept.rs   ::  marker\n")

            self.assertEqual(self.run_main(["--list", str(listed), "--root", str(root)]), 1)

    def test_main_refuses_to_pass_on_an_empty_list(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            listed = write_list(root, "# only comments\n\n")

            self.assertEqual(self.run_main(["--list", str(listed), "--root", str(root)]), 1)

    def test_main_fails_when_the_list_cannot_be_read(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)

            self.assertEqual(
                self.run_main(["--list", str(root / "absent"), "--root", str(root)]), 1
            )

    def test_main_writes_github_outputs(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            (root / "kept.rs").write_text("nothing here\n", encoding="utf-8")
            listed = write_list(root, "kept.rs   ::  marker\n")
            output = root / "github_output"

            exit_code = self.run_main(
                [
                    "--list",
                    str(listed),
                    "--root",
                    str(root),
                    "--github-output",
                    str(output),
                ]
            )

            self.assertEqual(exit_code, 1)
            text = output.read_text(encoding="utf-8")
            self.assertIn("missing_files<<GUARD_EOF\nGUARD_EOF\n", text)
            self.assertIn(
                "marker_missing<<GUARD_EOF\nkept.rs (missing marker: marker)\nGUARD_EOF\n", text
            )

    def test_main_reports_pass_state_for_intact_entries(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            (root / "kept.rs").write_text("// marker\n", encoding="utf-8")
            listed = write_list(root, "kept.rs   ::  marker\n")
            output = root / "github_output"

            exit_code = self.run_main(
                [
                    "--list",
                    str(listed),
                    "--root",
                    str(root),
                    "--github-output",
                    str(output),
                ]
            )

            self.assertEqual(exit_code, 0)
            text = output.read_text(encoding="utf-8")
            self.assertIn("missing_files<<GUARD_EOF\nGUARD_EOF\n", text)
            self.assertIn("marker_missing<<GUARD_EOF\nGUARD_EOF\n", text)


class RealListTests(unittest.TestCase):
    REPO_ROOT = Path(__file__).resolve().parent.parent

    @unittest.skipUnless(
        (REPO_ROOT / guard.DEFAULT_LIST).is_file(), "fork-owned list is absent"
    )
    def test_repository_list_is_satisfied(self) -> None:
        exit_code = self.run_main(
            ["--list", str(self.REPO_ROOT / guard.DEFAULT_LIST), "--root", str(self.REPO_ROOT)]
        )

        self.assertEqual(exit_code, 0)

    def run_main(self, args: list[str]) -> int:
        with patch.dict(os.environ, {}, clear=False):
            os.environ.pop("GITHUB_OUTPUT", None)
            return guard.main(args)


if __name__ == "__main__":
    unittest.main()
