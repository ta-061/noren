"""Offline regression tests for the release-notes generator.

These exercise the pure classification and rendering functions over canned
commit shapes; no git subprocess is invoked, so the suite runs anywhere.
"""

from __future__ import annotations

import unittest

from scripts.release import generate_notes as gn


class ParseSubjectTests(unittest.TestCase):
    def test_typed_scoped_subject(self) -> None:
        commit_type, scope, pr = gn.parse_subject("feat(ssh): raise host cap (#179)")
        self.assertEqual((commit_type, scope, pr), ("feat", "ssh", 179))

    def test_typed_unscoped_subject(self) -> None:
        commit_type, scope, pr = gn.parse_subject("fix: stop a crash")
        self.assertEqual((commit_type, scope, pr), ("fix", None, None))

    def test_pre_convention_subject_is_counted_not_listed(self) -> None:
        commit_type, scope, _pr = gn.parse_subject("Initial commit")
        self.assertIsNone(commit_type)
        self.assertIsNone(scope)

    def test_merge_subject_is_not_a_listed_commit(self) -> None:
        # Merge subjects reach build_pr_map, never parse_subject, but the
        # parser must not misread one as a conventional commit either.
        commit_type, _scope, _pr = gn.parse_subject("Merge pull request #4 from x/y")
        self.assertIsNone(commit_type)


class ClassifyTests(unittest.TestCase):
    def commits(self, subjects: list[str]) -> list[dict]:
        return [
            {"sha": f"{index:040x}", "subject": subject}
            for index, subject in enumerate(subjects)
        ]

    def test_user_visible_types_are_listed_with_areas(self) -> None:
        classified = gn.classify(
            self.commits(
                [
                    "feat(app): draw the sidebar",
                    "fix(pty): reap the child",
                    "perf(terminal): fewer copies",
                ]
            ),
            {},
        )
        areas = sorted(item["area"] for item in classified["listed"])
        self.assertEqual(
            areas,
            ["PTY and process layer", "Terminal emulation core", "Workspace app and window"],
        )

    def test_internal_scopes_are_counted_even_when_type_is_visible(self) -> None:
        classified = gn.classify(
            self.commits(["fix(test): pin the oracle", "feat(bench): add harness"]),
            {},
        )
        self.assertEqual(classified["listed"], [])
        self.assertEqual(sum(classified["counted"].values()), 2)

    def test_unmapped_scope_fails_loudly(self) -> None:
        with self.assertRaises(SystemExit):
            gn.classify(self.commits(["feat(mystery): a new scope appears"]), {})

    def test_inline_pr_number_is_kept(self) -> None:
        classified = gn.classify(self.commits(["fix(app): a thing (#76)"]), {})
        self.assertEqual(classified["listed"][0]["pr"], 76)

    def test_legacy_scope_as_type_is_promoted_to_feat(self) -> None:
        classified = gn.classify(
            self.commits(["theme: built-in palettes with measured contrast"]), {}
        )
        self.assertEqual(len(classified["listed"]), 1)
        self.assertEqual(classified["listed"][0]["type"], "feat")
        self.assertEqual(classified["counted"], {})

    def test_counted_label_keeps_the_scope(self) -> None:
        classified = gn.classify(self.commits(["fix(test): pin the oracle"]), {})
        self.assertEqual(classified["counted"], {"fix(test)": 1})

    def test_pr_map_supplies_number_when_subject_has_none(self) -> None:
        commits = self.commits(["fix(app): a thing"])
        classified = gn.classify(commits, {commits[0]["sha"]: 42})
        self.assertEqual(classified["listed"][0]["pr"], 42)


class RenderTests(unittest.TestCase):
    def test_safe_subject_neutralizes_markdown_links(self) -> None:
        guarded = gn.safe_subject("fix(docs): say [see here](https://example.invalid)")
        self.assertNotIn("](https://", guarded)

    def test_rendered_notes_end_with_newline_and_link_targets_exist(self) -> None:
        commits = [
            {"sha": "a" * 40, "subject": "feat(app): one feature"},
            {"sha": "b" * 40, "subject": "docs: a doc commit"},
            {"sha": "c" * 40, "subject": "Establish Discovery and governance baseline"},
        ]
        text = gn.render(
            "0.1.0-preview",
            "d" * 40,
            commits,
            {},
            "Merge pull request #1 from x/y",
            "2026-08-28",
            "e" * 40,
        )
        self.assertTrue(text.endswith("\n"))
        for line in text.splitlines():
            self.assertFalse(line.endswith((" ", "\t")), msg=repr(line))
        # The banner links to real files in the repository.
        self.assertIn("known-limitations.md", text)
        self.assertIn("D-M8-001-preview-scope.md", text)
        self.assertIn("3 non-merge commits", text)  # total count line
        self.assertIn("feat(app): one feature (aaaaaaa)", text)
        self.assertIn("- docs: 1 commits", text)
        self.assertIn(
            "early project history (pre-convention subjects): 1 commits", text
        )


if __name__ == "__main__":
    unittest.main()
