"""Measure one owned child, never system-wide or unrelated processes."""
import os
import signal
import subprocess
import sys
import time


def measured_run(cmd, workspace, env, log, timeout=1200):
    if timeout <= 0 or not hasattr(os, "wait4") or sys.platform not in ("darwin", "linux"):
        raise ValueError("positive timeout and supported Unix wait4 platform required")
    started = time.monotonic()
    with subprocess.Popen(cmd, cwd=workspace, env=env, stdout=log,
                          stderr=subprocess.STDOUT, start_new_session=True) as process:
        try:
            while True:
                pid, status, usage = os.wait4(process.pid, os.WNOHANG)
                if pid:
                    process.returncode = os.waitstatus_to_exitcode(status)
                    return dict(exit_code=process.returncode, pid=pid,
                                wall_sec=time.monotonic()-started,
                                user_cpu_sec=usage.ru_utime, system_cpu_sec=usage.ru_stime,
                                maxrss_bytes=int(usage.ru_maxrss)*(1 if sys.platform == "darwin" else 1024),
                                major_faults=usage.ru_majflt,
                                block_input=usage.ru_inblock, block_output=usage.ru_oublock,
                                measurement="wait4_child_high_water_not_concurrent_tree_sum",
                                cold_filesystem_cache=False)
                if time.monotonic()-started >= timeout:
                    raise subprocess.TimeoutExpired(cmd, timeout)
                time.sleep(0.05)
        except BaseException:
            # Includes descendants such as Cargo's test child. Never kill by name.
            try: os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError: pass
            try: process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                try: os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError: pass
                process.wait()
            raise


def select_profiles(configs, requested, label):
    if requested is None:
        return configs
    if not requested or len(requested) != len(set(requested)):
        raise ValueError("nonempty unique profile selection required")
    known = {label(c): c for c in configs}
    if set(requested)-known.keys():
        raise ValueError("profile outside the approved continuous runner scope")
    return [known[name] for name in requested]
