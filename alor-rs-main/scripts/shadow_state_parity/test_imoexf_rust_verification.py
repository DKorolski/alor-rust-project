"""IMOEXF acceptance must fail before writing a report on incomplete/tampered runs."""
from contextlib import contextmanager
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from compare import ParityError, canonical_bytes, file_hash
from verify_imoexf_rust import verify


class ImoexfRustVerificationTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        self.handoff, self.reference, self.candidate = [root / name for name in ("handoff", "reference", "candidate")]
        for directory in (self.handoff, self.reference, self.candidate):
            directory.mkdir()
        (self.handoff / "model_bars_07.csv").write_text("fixture\n")
        self.metadata = dict(status="GENERATED_NOT_YET_COMPARED", checks=[],
                             raw_file_sha256=file_hash(self.handoff / "model_bars_07.csv"))

    def run_check(self):
        (self.candidate / "run_metadata.json").write_bytes(canonical_bytes(self.metadata))
        return verify(self.handoff, self.reference, self.candidate)

    def test_reject_changed_raw_file(self):
        self.metadata["raw_file_sha256"] = "changed"
        with self.assertRaisesRegex(ParityError, "input file mismatch"):
            self.run_check()
        self.assertFalse((self.candidate / "verification.json").exists())

    def test_reject_partial_or_duplicate_sleeves(self):
        for sleeves in ([], ["mr"], ["combo", "mr"], ["mr", "mr"], ["mr", "combo", "combo"]):
            self.metadata["checks"] = [dict(sleeve=sleeve) for sleeve in sleeves]
            with self.assertRaisesRegex(ParityError, "coverage mismatch"):
                self.run_check()

    def test_reject_unknown_run_status(self):
        self.metadata["status"] = "PARTIAL"
        with self.assertRaisesRegex(ParityError, "run status"):
            self.run_check()

    def test_no_overwrite_of_existing_report(self):
        path = self.candidate / "verification.json"
        path.write_text("keep\n")
        with self.assertRaisesRegex(ParityError, "already exists"):
            self.run_check()
        self.assertEqual(path.read_text(), "keep\n")

    def test_reject_changed_trace_without_comparison(self):
        @contextmanager
        def reference(_):
            yield None, None, SimpleNamespace(check_manifest=lambda: None)

        ref = self.reference / "mr"
        actual = self.candidate / "mr"
        ref.mkdir()
        actual.mkdir()
        (ref / "manifest.json").write_text("{}")
        (actual / "actual_state_trace.jsonl").write_text("modified")
        self.metadata["checks"] = [dict(sleeve="mr", manifest_sha256=file_hash(ref / "manifest.json"),
                                        trace_sha256="old"), dict(sleeve="combo")]
        with patch("verify_imoexf_rust.reference_modules", reference):
            with patch("verify_imoexf_rust.compare_traces") as compare:
                with self.assertRaisesRegex(ParityError, "provenance mismatch"):
                    self.run_check()
                compare.assert_not_called()
        self.assertFalse((self.candidate / "verification.json").exists())


if __name__ == "__main__":
    unittest.main()
