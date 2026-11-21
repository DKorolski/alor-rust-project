import hashlib
from pathlib import Path
import tempfile
import unittest

import freeze_imoexf_calendar_oi_stage1 as freeze


class BaselineTests(unittest.TestCase):
    def test_conflicting_dependency_versions_cannot_be_silently_resealed(self):
        expected = {}
        freeze.pin(expected, 'model.py', 'a' * 64)
        freeze.pin(expected, 'model.py', 'a' * 64)
        with self.assertRaisesRegex(ValueError, 'conflicting'):
            freeze.pin(expected, 'model.py', 'b' * 64)

    def test_invalid_hash_is_rejected(self):
        for invalid in ['', 'X' * 64, '0' * 63]:
            with self.assertRaises(ValueError):
                freeze.pin({}, 'model.py', invalid)

    def test_hashing_does_not_modify_a_reference(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'reference'
            path.write_bytes(b'frozen reference')
            self.assertEqual(freeze.digest(path), hashlib.sha256(b'frozen reference').hexdigest())
            self.assertEqual(path.read_bytes(), b'frozen reference')

    def test_outside_workspace_reference_is_not_read(self):
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary) / 'workspace'; workspace.mkdir()
            path = Path(temporary) / 'foreign'; path.write_bytes(b'not a dependency')
            with self.assertRaises(ValueError):
                freeze.scoped_file(workspace, path)

    def test_symlink_reference_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary).resolve()
            path = workspace / 'reference'; path.write_bytes(b'frozen')
            link = workspace / 'alias'; link.symlink_to(path)
            with self.assertRaises(ValueError):
                freeze.scoped_file(workspace, link)


if __name__ == '__main__':
    unittest.main()
