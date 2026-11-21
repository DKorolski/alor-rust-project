import hashlib
from pathlib import Path
import tempfile
import unittest

import export_shadow_calendar_reference as exporter


class ReferenceExportTests(unittest.TestCase):
    def test_pinned_reference_is_read_without_modification(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "reference.py"
            original = b"# immutable fixture\n"
            path.write_bytes(original)
            self.assertEqual(exporter.pinned(path, hashlib.sha256(original).hexdigest()), original)
            self.assertEqual(path.read_bytes(), original)

    def test_changed_reference_cannot_be_resealed(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "reference.py"
            path.write_bytes(b"changed source")
            with self.assertRaisesRegex(ValueError, "frozen reference changed"):
                exporter.pinned(path, "0" * 64)

    def test_fixture_windows_are_historical_not_an_inferred_future_calendar(self):
        for _, first, last in exporter.WINDOWS:
            self.assertLessEqual(first, last)
            self.assertLessEqual(last, "2026-11-15")


if __name__ == "__main__":
    unittest.main()
