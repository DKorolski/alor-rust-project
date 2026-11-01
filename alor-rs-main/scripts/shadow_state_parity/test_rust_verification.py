"""Candidate provenance failures must not leave an acceptance report."""
from contextlib import contextmanager
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from compare import ParityError, canonical_bytes, file_hash
from verify_usdrubf_rust import verify


class RustVerificationTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.handoff, self.reference, self.candidate = [self.root / name for name in ("handoff", "reference", "candidate")]
        for directory in (self.handoff, self.reference, self.candidate):
            directory.mkdir()
        (self.handoff / "raw_model_input.csv").write_text("fixture\n")
        self.metadata = dict(status="GENERATED_NOT_YET_COMPARED", checks=[],
                             raw_file_sha256=file_hash(self.handoff / "raw_model_input.csv"))

    def run_check(self):
        (self.candidate / "run_metadata.json").write_bytes(canonical_bytes(self.metadata))
        return verify(self.handoff, self.reference, self.candidate)

    def test_changed_raw_file_is_not_accepted(self):
        self.metadata["raw_file_sha256"] = "wrong"
        with self.assertRaisesRegex(ParityError, "input file mismatch"):
            self.run_check()
        self.assertFalse((self.candidate / "verification.json").exists())

    def test_unknown_run_status_is_not_accepted(self):
        self.metadata["status"] = "PARTIAL_RUN"
        with self.assertRaisesRegex(ParityError, "run status"):
            self.run_check()
        self.assertFalse((self.candidate / "verification.json").exists())

    def test_missing_profile_cannot_pass_empty_comparison(self):
        @contextmanager
        def reference(_):
            yield None, None, SimpleNamespace(integrity=lambda: dict(profiles=[dict(engine=dict(model_id="required"))]))

        with patch("verify_usdrubf_rust.reference_modules", reference):
            with self.assertRaisesRegex(ParityError, "coverage mismatch"):
                self.run_check()
        self.assertFalse((self.candidate / "verification.json").exists())

    def test_existing_acceptance_report_is_not_overwritten(self):
        report = self.candidate / "verification.json"
        report.write_text("preserve existing report\n")
        with self.assertRaisesRegex(ParityError, "already exists"):
            self.run_check()
        self.assertEqual(report.read_text(), "preserve existing report\n")


if __name__ == "__main__":
    unittest.main()
