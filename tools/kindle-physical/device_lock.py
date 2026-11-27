"""Hold the verification worker's one physical Kindle lock in the operator process."""

from contextlib import contextmanager
import fcntl
import os
from pathlib import Path
import stat
import threading


_LOCK_PATH = Path('/var/lib/kindle-physical/device.lock')
_fork_guard = threading.Lock()
_open_locks = set()


class DeviceBusy(RuntimeError):
    """The physical Kindle operation lock is already owned."""


class DeviceLockError(RuntimeError):
    """The canonical lock could not be opened or verified."""


def _after_fork_in_child():
    # Do not unlock the parent's shared open file description from the child.
    for fd in _open_locks:
        os.close(fd)
    _open_locks.clear()
    _fork_guard.release()


os.register_at_fork(before=_fork_guard.acquire, after_in_parent=_fork_guard.release,
                    after_in_child=_after_fork_in_child)


def _identity(info, kind, mode):
    if (stat.S_IFMT(info.st_mode) != kind or stat.S_IMODE(info.st_mode) != mode
            or info.st_uid != 0 or (kind == stat.S_IFREG and info.st_nlink != 1)):
        raise DeviceLockError('Unexpected physical Kindle lock type, owner, mode, or link count')
    return info.st_dev, info.st_ino


def _verify(directory, fd):
    parent = _identity(os.fstat(directory), stat.S_IFDIR, 0o700)
    named_parent = _identity(os.stat(_LOCK_PATH.parent, follow_symlinks=False), stat.S_IFDIR, 0o700)
    opened = _identity(os.fstat(fd), stat.S_IFREG, 0o600)
    named = _identity(os.stat(_LOCK_PATH.name, dir_fd=directory, follow_symlinks=False), stat.S_IFREG, 0o600)
    if parent != named_parent or opened != named:
        raise DeviceLockError('Physical Kindle lock directory or file identity changed')


@contextmanager
def operation_lock():
    """Acquire without waiting; keep the scope through device cleanup and handback."""
    pid = os.getpid()
    fd = None
    try:
        try:
            directory = os.open(_LOCK_PATH.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
            try:
                _identity(os.fstat(directory), stat.S_IFDIR, 0o700)
                with _fork_guard:
                    fd = os.open(_LOCK_PATH.name, os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW | os.O_CLOEXEC,
                                 dir_fd=directory)
                    _open_locks.add(fd)
                _verify(directory, fd)
                try:
                    fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                except BlockingIOError:
                    raise DeviceBusy('Physical Kindle is busy; no device operation started') from None
                _verify(directory, fd)
            finally:
                os.close(directory)
        except OSError as error:
            raise DeviceLockError('Physical Kindle operation lock is unavailable') from error
        yield
    finally:
        # A Python fork closes its inherited copy above; only the owner may
        # close this fd here. CLOEXEC protects non-Python transport execs.
        if fd is not None and os.getpid() == pid:
            with _fork_guard:
                _open_locks.discard(fd)
                os.close(fd)
