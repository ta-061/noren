"""Offline regression tests for the release-notes generator.

These exercise the pure classification and rendering functions over canned
commit shapes; no git subprocess is invoked, so the suite runs anywhere. The
build_pr_map tests patch `git` itself (still no subprocess is spawned) because
the two merge-subject shapes and the loud unknown-format guard are the
regressions this suite exists to pin: the first generator silently dropped
every `Merge PR #N:` coordinator merge.
"""

from __future__ import annotations

import unittest
from unittest import mock

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


class MergeSubjectTests(unittest.TestCase):
    def test_github_button_form_is_credited(self) -> None:
        self.assertEqual(
            gn.merge_pr_number("Merge pull request #62 from ta-061/x"), 62
        )

    def test_coordinator_form_is_credited(self) -> None:
        # The shape the first generator silently dropped (real example: the
        # merge of PR #62, sha 4fe5f6f).
        self.assertEqual(
            gn.merge_pr_number("Merge PR #62: snap cursor off continuations"), 62
        )

    def test_non_pr_merge_is_not_credited(self) -> None:
        self.assertIsNone(gn.merge_pr_number("Merge branch 'main' into x"))

    def test_subsumed_list_is_parsed(self) -> None:
        subject = "Merge PR #29: Terminal Core stack (subsumes #21, #23, #31, #30)"
        self.assertEqual(gn.subsumed_pr_numbers(subject), [21, 23, 31, 30])

    def test_subsumed_list_absent_is_empty(self) -> None:
        self.assertEqual(gn.subsumed_pr_numbers("Merge PR #29: no stack here"), [])


class BuildPrMapTests(unittest.TestCase):
    def spine_git(self, spine: str, brought: list[str]):
        def fake_git(*args: str) -> str:
            if args[0] == "log":
                return spine
            assert args[0] == "rev-list", args
            return brought.pop(0)

        return fake_git

    def test_both_merge_formats_credit_their_pr(self) -> None:
        spine = (
            "a" * 40
            + "\x00"
            + "p" * 40
            + " "
            + "b" * 40
            + "\x00"
            + "Merge pull request #62 from x/y\n"
            + "c" * 40
            + "\x00"
            + "q" * 40
            + " "
            + "d" * 40
            + "\x00"
            + "Merge PR #63: a coordinator merge\n"
        )
        with mock.patch.object(
            gn, "git", side_effect=self.spine_git(spine, ["e" * 40 + "\n", "f" * 40 + "\n"])
        ):
            pr_map, subsumed = gn.build_pr_map("HEAD")
        self.assertEqual(pr_map, {"e" * 40: 62, "f" * 40: 63})
        self.assertEqual(subsumed, set())

    def test_subsumed_merged_prs_are_returned_for_counting(self) -> None:
        spine = (
            "a" * 40
            + "\x00"
            + "p" * 40
            + " "
            + "b" * 40
            + "\x00"
            + "Merge PR #29: Terminal Core stack (subsumes #21, #23, #31, #30)\n"
        )
        with mock.patch.object(
            gn, "git", side_effect=self.spine_git(spine, ["" ])
        ):
            pr_map, subsumed = gn.build_pr_map("HEAD")
        self.assertEqual(pr_map, {})
        # Only the entries gh reports MERGED count (#23, #31 are only CLOSED).
        self.assertEqual(subsumed, {21, 30})

    def test_unknown_merge_format_fails_loudly(self) -> None:
        spine = (
            "a" * 40
            + "\x00"
            + "p" * 40
            + " "
            + "b" * 40
            + "\x00"
            + "Merge whatever #62 via some new tool\n"
        )
        with mock.patch.object(gn, "git", side_effect=self.spine_git(spine, [])):
            with self.assertRaises(SystemExit):
                gn.build_pr_map("HEAD")


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

    def test_inline_issue_reference_is_not_pr_credit(self) -> None:
        # #59 and #153 are issues (gh api pulls/N is 404); their trailing
        # "(#N)" must not be counted as a landed pull request.
        classified = gn.classify(self.commits(["ci: cache the build (#153)"]), {})
        self.assertEqual(classified["pr_numbers"], set())

    def test_inline_issue_ref_falls_back_to_pr_map(self) -> None:
        commits = self.commits(["fix(app): closes the bug (#59)"])
        classified = gn.classify(commits, {commits[0]["sha"]: 70})
        self.assertEqual(classified["listed"][0]["pr"], 70)
        self.assertEqual(classified["pr_numbers"], {70})

    def test_subsumed_prs_count_as_distinct_landed(self) -> None:
        classified = gn.classify(self.commits(["feat(app): one feature"]), {}, {21, 30})
        self.assertEqual(classified["pr_numbers"], {21, 30})


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

    def test_pr_count_line_names_the_gh_audit(self) -> None:
        text = gn.render(
            "0.1.0-preview",
            "d" * 40,
            [ {"sha": "a" * 40, "subject": "feat(app): one feature"}],
            {},
            "Merge pull request #1 from x/y",
            "2026-08-28",
            "e" * 40,
        )
        self.assertIn(
            "distinct pull requests (count verified against "
            "`gh pr list --state merged`", text
        )


if __name__ == "__main__":
    unittest.main()
