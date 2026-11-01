import copy
import subprocess
import tempfile
import unittest
from pathlib import Path

from linux_release import (PROFILES, SCHEMA, checked_copy, digest, source_files, source_provenance,
                           validate_release_metadata, validate_snapshot, write)


class LinuxReleaseProbeTests(unittest.TestCase):
    def git_fixture(self, root):
        def git(*args):
            subprocess.run(["git", "-c", "core.hooksPath=/dev/null", *args], cwd=root,
                           check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        git("init")
        git("config", "user.name", "Fixture")
        git("config", "user.email", "fixture@example.invalid")
        workspace = root / "workspace"
        workspace.mkdir()
        (workspace / "Cargo.lock").write_text("pinned\n")
        (root / ".gitignore").write_text("ignored.rs\n")
        git("add", ".")
        git("commit", "-m", "fixture")
        return workspace, git

    def test_clean_commit_provenance_in_nested_workspace(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace, _ = self.git_fixture(root)
            result = source_provenance(workspace, {"Cargo.lock": digest(workspace / "Cargo.lock")})
            self.assertEqual(result["source_kind"], "manifest_pinned_clean_commit_snapshot")
            self.assertTrue(result["git_worktree_clean"])
            self.assertEqual(result["uncommitted_source_paths"], [])

    def test_dirty_and_staged_sources_are_not_clean_commit(self):
        with tempfile.TemporaryDirectory() as directory:
            workspace, git = self.git_fixture(Path(directory))
            (workspace / "Cargo.lock").write_text("changed\n")
            for staged in (False, True):
                if staged:
                    git("add", ".")
                result = source_provenance(workspace, {"Cargo.lock": digest(workspace / "Cargo.lock")})
                self.assertFalse(result["git_worktree_clean"])
                self.assertEqual(result["uncommitted_source_paths"], ["Cargo.lock"])
                self.assertIn("dirty_worktree", result["source_kind"])

    def test_ignored_source_is_not_part_of_clean_commit(self):
        with tempfile.TemporaryDirectory() as directory:
            workspace, _ = self.git_fixture(Path(directory))
            path = workspace / "ignored.rs"
            path.write_text("not committed\n")
            result = source_provenance(workspace, {"ignored.rs": digest(path)})
            self.assertTrue(result["git_worktree_clean"])
            self.assertEqual(result["uncommitted_source_paths"], ["ignored.rs"])
            self.assertIn("dirty_worktree", result["source_kind"])

    def test_unrelated_untracked_file_blocks_clean_claim(self):
        with tempfile.TemporaryDirectory() as directory:
            workspace, _ = self.git_fixture(Path(directory))
            (workspace / "notes.txt").write_text("uncommitted\n")
            result = source_provenance(workspace, {"Cargo.lock": digest(workspace / "Cargo.lock")})
            self.assertFalse(result["git_worktree_clean"])
            self.assertEqual(result["uncommitted_source_paths"], [])
            self.assertIn("dirty_worktree", result["source_kind"])

    def bundle(self, root):
        (root / "source").mkdir()
        src = root / "source/Cargo.toml"
        src.write_text("fixture")
        fixtures = {}
        for profile in PROFILES:
            parent = root / "fixtures" / profile
            parent.mkdir(parents=True)
            for name in ("input.json", "expected.jsonl"):
                path = parent / name
                path.write_text("{}\n")
                fixtures[str(path.relative_to(root / "fixtures"))] = digest(path)
        return dict(schema=SCHEMA, sources={"Cargo.toml":digest(src)}, fixtures=fixtures,
                    expected_stats={p:{} for p in PROFILES})

    def test_pinned_snapshot_and_tamper(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            manifest = self.bundle(root)
            write(root / "manifest.json", manifest)
            self.assertEqual(validate_snapshot(root), manifest)
            (root / "source/Cargo.toml").write_text("changed")
            with self.assertRaisesRegex(ValueError, "hash mismatch"):
                validate_snapshot(root)

    def test_manifest_scope_and_path_escape(self):
        for mutation in (lambda m:m.update(sources={}),
                         lambda m:m["fixtures"].pop(next(iter(m["fixtures"]))),
                         lambda m:m["expected_stats"].clear(),
                         lambda m:m["sources"].update({"../outside":"abc"})):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                root = Path(directory).resolve()
                manifest = self.bundle(root)
                mutation(manifest)
                write(root / "manifest.json", manifest)
                with self.assertRaises(ValueError): validate_snapshot(root)

    def test_symlink_parent_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            manifest = self.bundle(root)
            (root / "source").rename(root / "external")
            (root / "source").symlink_to(root / "external", target_is_directory=True)
            write(root / "manifest.json", manifest)
            with self.assertRaisesRegex(ValueError, "invalid snapshot"):
                validate_snapshot(root)

    def test_copy_hash_bound_and_no_overwrite(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            src, dst = root / "source", root / "copy"
            src.write_bytes(b"abc")
            with self.assertRaises(ValueError): checked_copy(src, dst, 2)
            with self.assertRaises(ValueError): checked_copy(src, dst, 3, "wrong")
            self.assertFalse(dst.exists())
            self.assertEqual(checked_copy(src, dst, 3, digest(src)), digest(src))
            with self.assertRaises(FileExistsError): checked_copy(src, dst, 3)
            with self.assertRaises(FileExistsError): write(dst, {})

    def test_source_selection_excludes_secret_and_live_artifacts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in (".env", "docs/private.json", "target/secret.rs",
                         "strategy-runtime/src/model.rs", "strategy-runtime/src/.env"):
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("fixture")
            selected = {str(p.relative_to(root)) for p in source_files(root)}
            self.assertIn("strategy-runtime/src/model.rs", selected)
            self.assertFalse(selected & {".env", "docs/private.json", "target/secret.rs",
                                          "strategy-runtime/src/.env"})

    def metadata(self):
        bins = dict(target="x86_64-unknown-linux-gnu", profile="release", debug_assertions=False)
        report = dict(os="linux", arch="x86_64", debug_assertions=False,
                      production_receipts_certified=False, deployment=False)
        restored = dict(report, new_process=True, cold_filesystem_cache=False,
                        witness=dict(status=dict(order_emission_allowed=False, session_complete=True)))
        exported = dict(exit_code=0, pid=100, maxrss_bytes=1024, wall_sec=1.0,
                        cold_filesystem_cache=False)
        return [bins, report, restored, exported, dict(exported, pid=101)]

    def test_release_and_separate_process_evidence(self):
        validate_release_metadata(*self.metadata())
        mutations = [(0, "profile", "test"), (0, "debug_assertions", True),
                     (1, "os", "macos"), (1, "arch", "aarch64"),
                     (1, "debug_assertions", True), (2, "new_process", False),
                     (2, "cold_filesystem_cache", True), (2, "deployment", True),
                     (2, "production_receipts_certified", True), (4, "pid", 100),
                     (4, "exit_code", 1), (4, "maxrss_bytes", 0),
                     (4, "wall_sec", float("nan")), (4, "cold_filesystem_cache", True)]
        for index, key, value in mutations:
            with self.subTest(key=key, value=value):
                items = copy.deepcopy(self.metadata())
                items[index][key] = value
                with self.assertRaises(ValueError): validate_release_metadata(*items)
        for key, value in (("order_emission_allowed", True), ("session_complete", False)):
            items = self.metadata()
            items[2]["witness"]["status"][key] = value
            with self.assertRaises(ValueError): validate_release_metadata(*items)


if __name__ == "__main__": unittest.main()
