import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from process_probe import measured_run, select_profiles


class ProcessProbeTests(unittest.TestCase):
    def test_profile_scope_and_duplicates(self):
        configs=[dict(name="a"),dict(name="b")]
        label=lambda c:c["name"]
        self.assertEqual(select_profiles(configs,None,label),configs)
        self.assertEqual(select_profiles(configs,["b"],label),[configs[1]])
        for values in ([],["a","a"],["not-approved"]):
            with self.assertRaises(ValueError): select_profiles(configs,values,label)

    def test_child_metrics_and_exit_status(self):
        with tempfile.TemporaryDirectory() as directory:
            with (Path(directory)/"log").open("x") as log:
                result=measured_run([sys.executable,"-c","import sys; sys.exit(7)"],directory,os.environ.copy(),log)
        self.assertEqual(result["exit_code"],7)
        self.assertGreater(result["maxrss_bytes"],0)
        self.assertGreater(result["wall_sec"],0)
        self.assertFalse(result["cold_filesystem_cache"])

    def test_timeout_reaps_owned_child(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)
            command=[sys.executable,"-c","import os, pathlib, time; pathlib.Path('pid').write_text(str(os.getpid())); time.sleep(60)"]
            with (path/"log").open("x") as log:
                with self.assertRaises(subprocess.TimeoutExpired):
                    measured_run(command,directory,os.environ.copy(),log,timeout=0.5)
            pid=int((path/"pid").read_text())
            with self.assertRaises(ProcessLookupError): os.kill(pid,0)


if __name__ == "__main__": unittest.main()
