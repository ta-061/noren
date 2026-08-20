"""Regression tests for the Milestone 8 release tooling.

Each test names the behaviour it pins so that removing the behaviour fails
the test; the mutations used to prove that are recorded in
docs/release/README.md. Pure functions are tested with fixed inputs; the
two integration tests exercise the real repository history.
"""

from __future__ import annotations

import shutil
import tempfile
import unittest
from pathlib import Path

from scripts.release import build, notes

# sha256("hello\n") — a fixed external constant, not a call back into the
# code under test.
HELLO_N_SHA256 = "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03"


def _scratch_dir(test: unittest.TestCase) -> Path:
    path = Path(tempfile.mkdtemp(prefix="noren-release-tests-"))
    test.addCleanup(shutil.rmtree, path, ignore_errors=True)
    return path


class ClassifySubjectTests(unittest.TestCase):
    def test_conventional_subject_with_scope(self) -> None:
        self.assertEqual(
            ("feat", "app: draw SGR background colour behind glyphs (#114) (#121)"),
            notes.classify_subject(
                "feat(app): draw SGR background colour behind glyphs (#114) (#121)"
            ),
        )

    def test_conventional_subject_without_scope(self) -> None:
        self.assertEqual(
            ("fix", "stop losing bytes under load (#42)"),
            notes.classify_subject("fix: stop losing bytes under load (#42)"),
        )

    def test_breaking_mark_is_preserved(self) -> None:
        self.assertEqual(
            ("feat", "cli!: remove the legacy config path"),
            notes.classify_subject("feat(cli)!: remove the legacy config path"),
        )

    def test_unknown_subject_lands_in_other_verbatim(self) -> None:
        subject = "wip: checkpoint PR 120 remediation for Mac handoff"
        self.assertEqual(("other", subject), notes.classify_subject(subject))

    def test_non_merge_rejects_merge_shapes(self) -> None:
        for subject in (
            "Merge pull request #131 from ta-061/refactor/split-ssh-config-tests-123",
            "Merge PR #74: add the session domain contract and lifecycle model",
            "Merge remote-tracking branch 'origin/main' into agent/ssh-sidebar-hosts",
            "Merge branch 'main' into agent/session-restored-state",
        ):
            self.assertTrue(notes.is_merge_subject(subject), msg=subject)
        self.assertFalse(notes.is_merge_subject("fix(app): real subject"))


class GroupSubjectsTests(unittest.TestCase):
    def test_fixed_group_order_features_before_fixes(self) -> None:
        sections = notes.group_subjects(
            ["fix(app): second", "feat(app): first"]
        )
        self.assertEqual(
            [("feat", "Features"), ("fix", "Fixes")],
            [(key, title) for key, title, _entries in sections],
        )

    def test_every_subject_renders_exactly_once(self) -> None:
        sections = notes.group_subjects(
            [
                "feat(app): one",
                "fix(ssh): two",
                "docs: three",
                "wip: not conventional",
            ]
        )
        rendered = [entry for _k, _t, entries in sections for entry in entries]
        self.assertEqual(
            ["app: one", "ssh: two", "three", "wip: not conventional"],
            sorted(rendered),
        )

    def test_other_group_comes_last(self) -> None:
        sections = notes.group_subjects(["wip: odd", "feat(app): even"])
        self.assertEqual("other", sections[-1][0])


class ResolveBaselineTests(unittest.TestCase):
    def test_override_wins_over_tag_and_fallback(self) -> None:
        ref, source = notes.resolve_baseline("deadbee", ["v0.2.0"])
        self.assertEqual("deadbee", ref)
        self.assertIn("--since", source)

    def test_newest_tag_used_when_no_override(self) -> None:
        ref, source = notes.resolve_baseline(None, ["v0.2.0", "v0.1.0"])
        self.assertEqual("v0.2.0", ref)
        self.assertIn("git tag", source)

    def test_documented_fallback_when_no_tags_exist(self) -> None:
        ref, source = notes.resolve_baseline(None, [])
        self.assertEqual(notes.FALLBACK_BASELINE, ref)
        self.assertIn("Milestone 2", source)


class RenderTests(unittest.TestCase):
    def test_render_lists_every_subject_and_counts(self) -> None:
        sections = notes.group_subjects(
            ["feat(app): one", "fix(app): two", "wip: three"]
        )
        text = notes.render("headsha", "basesha", "test source", sections,
                            merge_count=4, total_commits=7, generated_utc="now")
        for entry in ("app: one", "app: two", "wip: three"):
            self.assertIn(f"- {entry}", text)
        self.assertIn("(3 listed, 4 merge commits elided)", text)
        self.assertIn("`basesha`", text)
        self.assertIn("`headsha`", text)

    def test_render_contains_owner_checklist_and_signing_gap(self) -> None:
        text = notes.render("h", "b", "src", [], 0, 0, "now")
        self.assertIn("Owner to complete before any publication", text)
        self.assertIn("SIGNING GAP", text)
        self.assertIn("- [ ]", text)
        self.assertIn("unsigned", text)


class RealRepositoryHistoryTests(unittest.TestCase):
    """The integration seam: the generator against the actual repository."""

    def test_collected_history_has_no_merge_subjects(self) -> None:
        subjects, _head, _base, merges = notes.collect_subjects(
            build.ROOT, notes.FALLBACK_BASELINE
        )
        self.assertGreater(merges, 0,
                           "real history must contain merge commits to exercise skipping")
        for subject in subjects:
            self.assertFalse(subject.startswith("Merge "), msg=subject)
        self.assertGreater(len(subjects), 50)

    def test_generated_notes_cover_history_and_gap(self) -> None:
        text = notes.build_notes(build.ROOT, None)
        self.assertIn("GENERATED TEMPLATE", text)
        self.assertIn("SIGNING GAP", text)
        listed = text.count("\n- ")
        self.assertGreater(listed, 50)


class BuildCommandTests(unittest.TestCase):
    def test_release_build_is_locked_and_package_scoped(self) -> None:
        self.assertEqual(
            ["cargo", "build", "--release", "--locked", "-p", "noren-app"],
            build.cargo_build_command(),
        )


class ChecksumTests(unittest.TestCase):
    def test_sha256_file_against_known_digest(self) -> None:
        path = _scratch_dir(self) / "hello.txt"
        path.write_bytes(b"hello\n")
        self.assertEqual(HELLO_N_SHA256, build.sha256_file(path))

    def test_manifest_entry_uses_sha256sum_two_space_format(self) -> None:
        digest = "a" * 64
        self.assertEqual(
            digest + "  noren-0.1.0-aarch64-apple-darwin",
            build.manifest_entry(digest, "noren-0.1.0-aarch64-apple-darwin"),
        )

    def test_write_manifest_covers_every_artifact_except_itself(self) -> None:
        dist = _scratch_dir(self)
        binary = dist / "noren-0.1.0-aarch64-apple-darwin"
        binary.write_bytes(b"artifact bytes")
        provenance = dist / build.PROVENANCE_NAME
        provenance.write_text("provenance\n", encoding="utf-8")
        notes_file = dist / build.NOTES_NAME
        notes_file.write_text("notes\n", encoding="utf-8")
        manifest = build.write_manifest(dist, [binary, provenance, notes_file])
        lines = manifest.read_text(encoding="utf-8").splitlines()
        self.assertEqual(3, len(lines))
        names = [line.split("  ", 1)[1] for line in lines]
        self.assertEqual(
            [binary.name, notes_file.name, provenance.name], names
        )
        self.assertEqual(build.sha256_file(binary), lines[0].split("  ")[0])
        self.assertNotIn(build.MANIFEST_NAME, names)


class NamingTests(unittest.TestCase):
    def test_artifact_name_format(self) -> None:
        self.assertEqual(
            "noren-0.1.0-aarch64-apple-darwin",
            build.artifact_name("0.1.0", "aarch64-apple-darwin"),
        )

    def test_host_triple_parsed_from_rustc_vv(self) -> None:
        rustc_vv = (
            "rustc 1.88.0 (6b00bc388 2025-06-23)\n"
            "binary: rustc\n"
            "host: aarch64-apple-darwin\n"
            "release: 1.88.0\n"
        )
        self.assertEqual("aarch64-apple-darwin", build.host_triple(rustc_vv))

    def test_version_comes_from_the_noren_app_package(self) -> None:
        metadata = (
            '{"packages":[{"name":"noren-pty","version":"0.9.9"},'
            '{"name":"noren-app","version":"0.1.0"},'
            '{"name":"noren-terminal","version":"0.8.8"}]}'
        )
        self.assertEqual("0.1.0", build.noren_app_version(metadata))


class GuardTests(unittest.TestCase):
    def test_non_macos_platform_is_refused(self) -> None:
        with self.assertRaises(SystemExit):
            build.assert_macos("linux")
        self.assertIsNone(build.assert_macos("darwin"))

    def test_tree_state_detects_changes(self) -> None:
        self.assertEqual("clean", build.tree_state(""))
        self.assertEqual("dirty", build.tree_state(" M crates/x\n"))


class StageBinaryTests(unittest.TestCase):
    def test_stage_binary_copies_content_and_sets_exec_bit(self) -> None:
        source_dir = _scratch_dir(self)
        dist = _scratch_dir(self)
        source = source_dir / "noren-app"
        source.write_bytes(b"\xcf\xfa\xed\xfe fake mach-o")
        staged = build.stage_binary(source, dist, "noren-0.1.0-aarch64-apple-darwin")
        self.assertEqual(dist / "noren-0.1.0-aarch64-apple-darwin", staged)
        self.assertEqual(b"\xcf\xfa\xed\xfe fake mach-o", staged.read_bytes())
        self.assertTrue(staged.stat().st_mode & 0o111,
                        "staged artifact must be executable")


if __name__ == "__main__":
    unittest.main()
