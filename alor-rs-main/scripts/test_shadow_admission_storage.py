import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

import shadow_admission_storage as storage


class AdmissionStorageTests(unittest.TestCase):
    def test_reproduces_mkdir_0750_becoming_0700_under_service_umask(self):
        with tempfile.TemporaryDirectory() as root:
            old = os.umask(0o077)
            try:
                path = Path(root)/'old-evidence'
                path.mkdir(mode=0o750)
            finally:
                os.umask(old)
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o700)
            self.assertEqual(path.stat().st_mode & stat.S_IRGRP, 0)
            self.assertEqual(path.stat().st_mode & stat.S_IXGRP, 0)

    def test_explicit_permissions_survive_strict_or_permissive_umask(self):
        for mask in (0o077, 0o027, 0o000):
            with self.subTest(mask=mask), tempfile.TemporaryDirectory() as root:
                path = Path(root)/'evidence'
                old = os.umask(mask)
                try:
                    storage.create_reviewed_directory(path, os.getgid(), owner_uid=os.getuid())
                finally:
                    os.umask(old)
                self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o750)
                self.assertEqual((path.stat().st_uid,path.stat().st_gid), (os.getuid(),os.getgid()))
                self.assertFalse(path.stat().st_mode & (stat.S_IWGRP|stat.S_IRWXO))

    def test_existing_or_symlinked_target_is_never_repaired_or_overwritten(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root)/'evidence'
            path.mkdir(mode=0o700)
            with self.assertRaises(FileExistsError):
                storage.create_reviewed_directory(path,os.getgid(),owner_uid=os.getuid())
            self.assertEqual(stat.S_IMODE(path.stat().st_mode),0o700)
            link = Path(root)/'link'
            link.symlink_to(path, target_is_directory=True)
            with self.assertRaises(FileExistsError):
                storage.create_reviewed_directory(link,os.getgid(),owner_uid=os.getuid())
            self.assertEqual(stat.S_IMODE(path.stat().st_mode),0o700)

    def test_symlinked_immediate_parent_is_rejected(self):
        with tempfile.TemporaryDirectory() as root:
            parent = Path(root)/'real'
            parent.mkdir()
            link = Path(root)/'link'
            link.symlink_to(parent, target_is_directory=True)
            with self.assertRaises(OSError):
                storage.create_reviewed_directory(link/'evidence',os.getgid(),owner_uid=os.getuid())
            self.assertFalse((parent/'evidence').exists())

    @unittest.skipUnless(sys.platform == 'linux' and os.geteuid() == 0,
                         'requires isolated Linux root with CHOWN/SETUID/SETGID')
    def test_linux_distinct_reader_uid_has_read_only_group_access(self):
        with tempfile.TemporaryDirectory() as root:
            base = Path(root)
            base.chmod(0o755)
            path = base/'evidence'
            old = os.umask(0o077)
            try:
                storage.create_reviewed_directory(path,10001)
                evidence = path/'natural_session.json'
                evidence.write_text('{}')
                os.chown(evidence,0,10001)
                evidence.chmod(0o440)
            finally:
                os.umask(old)
            probe = ('import os,sys; from pathlib import Path; '
                     'p=Path(sys.argv[1]); assert p.read_bytes()==b"{}"; '
                     'assert not os.access(p,os.W_OK); '
                     'assert not os.access(p.parent,os.W_OK)')
            allowed = subprocess.run([sys.executable,'-I','-B','-c',probe,str(evidence)],
                user=65534,group=10001,extra_groups=[],capture_output=True,timeout=10)
            denied = subprocess.run([sys.executable,'-I','-B','-c',probe,str(evidence)],
                user=65534,group=10002,extra_groups=[],capture_output=True,timeout=10)
            self.assertEqual(allowed.returncode,0,allowed.stderr.decode())
            self.assertNotEqual(denied.returncode,0)
            self.assertIn(b'PermissionError',denied.stderr)

    def test_ownership_failure_cannot_publish_group_readable_evidence(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root)/'evidence'
            with mock.patch.object(storage.os,'fchown',side_effect=PermissionError('controlled failure')):
                with self.assertRaises(PermissionError):
                    storage.create_reviewed_directory(path,os.getgid(),owner_uid=os.getuid())
            self.assertEqual(stat.S_IMODE(path.stat().st_mode),0o700)

    def test_invalid_identity_and_parent_traversal_are_rejected(self):
        for path,gid,uid in [('relative',os.getgid(),os.getuid()),('/tmp/x/../y',0,0),('/tmp/x',-1,0),('/tmp/x',0,-1)]:
            with self.assertRaises(ValueError):
                storage.create_reviewed_directory(path,gid,owner_uid=uid)

    def test_failure_diagnostics_preserve_class_but_not_tokens_paths_or_raw_output(self):
        stdout = json.dumps(dict(event='reaction_shadow_preflight_failed',error_class='PermissionError',
            admission_allowed=False,order_emission_allowed=False,secret='redis://user:secret@host/0'))
        result = storage.command_failure_details('/usr/bin/systemd-run',1,stdout,'Bearer secret-token')
        self.assertEqual(result['child_error_class'],'PermissionError')
        self.assertEqual(result['returncode'],1)
        self.assertEqual(result['program'],'systemd-run')
        text = json.dumps(result)
        for forbidden in ('redis://','secret-token','user:secret','Bearer','/usr/bin'):
            self.assertNotIn(forbidden,text)
        self.assertEqual(len(result['stdout_sha256']),64)

    def test_unknown_child_payload_is_not_echoed(self):
        result = storage.command_failure_details('runner',2,'not json','private details')
        self.assertNotIn('child_error_class',result)
        self.assertNotIn('private',json.dumps(result))


if __name__ == '__main__':
    unittest.main()
