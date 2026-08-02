"""Regression tests for the dependency-free documentation checker."""

from __future__ import annotations

import os
import subprocess
import unittest

from scripts import check_docs


class SecretPatternTests(unittest.TestCase):
    def assert_detected(self, prefix: str, suffix: str = "A" * 40) -> None:
        self.assertTrue(
            check_docs.contains_secret(prefix + suffix),
            msg=f"expected token prefix {prefix!r} to be detected",
        )

    def test_current_token_prefixes(self) -> None:
        for prefix in (
            "ghp_",
            "github_pat_",
            "sk-proj-",
            "sk-ant-api03-",
            "glpat-",
            "npm_",
            "xoxb-",
        ):
            with self.subTest(prefix=prefix):
                self.assert_detected(prefix)

    def test_aws_access_key_prefixes(self) -> None:
        for prefix in ("AKIA", "ASIA"):
            with self.subTest(prefix=prefix):
                self.assertTrue(check_docs.contains_secret(prefix + "A" * 16))

    def test_private_key_header(self) -> None:
        self.assertTrue(
            check_docs.contains_secret(
                "-----BEGIN OPENSSH " + "PRIVATE KEY-----\nredacted"
            )
        )

    def test_benign_text(self) -> None:
        self.assertFalse(
            check_docs.contains_secret(
                "Documentation mentions tokens but contains no credential value."
            )
        )


class RepositoryCheckTests(unittest.TestCase):
    def test_non_ascii_untracked_filename_is_enumerated(self) -> None:
        path = check_docs.ROOT / "docs" / "日本語-列挙テスト.md"
        self.assertFalse(path.exists())
        path.write_text("# Temporary\n", encoding="utf-8")
        self.addCleanup(path.unlink, missing_ok=True)
        self.assertIn(path, check_docs.repository_files())

    def test_link_cannot_escape_repository(self) -> None:
        failures: list[str] = []
        check_docs.check_local_links(
            check_docs.ROOT / "README.md",
            "[outside](../../../../../../etc/passwd)",
            failures,
        )
        self.assertEqual(1, len(failures))
        self.assertIn("escapes repository", failures[0])

    def test_secret_in_non_document_suffix_fails_main_check(self) -> None:
        path = check_docs.ROOT / "test-secret.env.example"
        self.assertFalse(path.exists())
        path.write_text("TOKEN=github_pat_" + "A" * 40 + "\n", encoding="utf-8")
        self.addCleanup(path.unlink, missing_ok=True)
        env = dict(os.environ, PYTHONDONTWRITEBYTECODE="1")
        completed = subprocess.run(
            ["python3", "scripts/check_docs.py"],
            cwd=check_docs.ROOT,
            env=env,
            capture_output=True,
            text=True,
        )
        self.assertNotEqual(0, completed.returncode)
        self.assertIn(path.name, completed.stderr)


if __name__ == "__main__":
    unittest.main()
