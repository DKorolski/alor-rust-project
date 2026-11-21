"""Cross-UID reviewed evidence staging; no grant, service or trading operations."""
import hashlib
import json
import os
from pathlib import Path
import re
import stat


def create_reviewed_directory(path, group_id, *, owner_uid=0):
    """Create NEW root-owned 0750 evidence, independently of the launcher's umask.

    Failure leaves the new incomplete directory for audit, never overwrites or
    recursively repairs an existing admission. The controller must HOLD on error.
    """
    path = Path(path)
    if (not path.is_absolute() or '..' in path.parts or not path.name
            or type(owner_uid) is not int or owner_uid < 0
            or type(group_id) is not int or group_id < 0):
        raise ValueError('invalid_reviewed_directory_identity')
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
    parent = os.open(path.parent, flags)
    directory = None
    try:
        os.mkdir(path.name, mode=0o700, dir_fd=parent)
        directory = os.open(path.name, flags, dir_fd=parent)
        os.fchown(directory, owner_uid, group_id)
        os.fchmod(directory, 0o750)
        info = os.fstat(directory)
        if (not stat.S_ISDIR(info.st_mode) or stat.S_IMODE(info.st_mode) != 0o750
                or info.st_uid != owner_uid or info.st_gid != group_id):
            raise RuntimeError('reviewed_directory_metadata_mismatch')
        os.fsync(directory)
        os.fsync(parent)
    finally:
        if directory is not None:
            os.close(directory)
        os.close(parent)


def command_failure_details(program, returncode, stdout, stderr):
    """Safe diagnostics: hashes/class only, never argv, credentials or raw output."""
    name = Path(program).name
    if not re.fullmatch(r'[A-Za-z0-9_.-]{1,64}', name):
        name = 'subprocess'
    data = [x if isinstance(x, bytes) else x.encode('utf-8', errors='replace') for x in (stdout, stderr)]
    result = dict(program=name, returncode=returncode,
        stdout_bytes=len(data[0]), stderr_bytes=len(data[1]),
        stdout_sha256=hashlib.sha256(data[0]).hexdigest(), stderr_sha256=hashlib.sha256(data[1]).hexdigest())
    for raw in data:
        for line in raw[-65536:].splitlines()[-8:]:
            try:
                record = json.loads(line)
            except (ValueError, UnicodeError):
                continue
            if (isinstance(record, dict) and record.get('event') == 'reaction_shadow_preflight_failed'
                    and record.get('order_emission_allowed') is False and record.get('admission_allowed') is False):
                error_class = record.get('error_class')
                if isinstance(error_class, str) and re.fullmatch(r'[A-Za-z][A-Za-z0-9_]{0,63}', error_class):
                    result['child_error_class'] = error_class
    return result
