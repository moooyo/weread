use std::{io, mem::MaybeUninit};

pub(super) fn guard_benchmark(expected_parent: libc::pid_t) -> io::Result<()> {
    // SAFETY: all prctl arguments are integers. The PID is compared only;
    // no signal is sent to any external PID or process group.
    unsafe {
        if expected_parent < 2 || libc::getppid() != expected_parent {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        let core = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if libc::setrlimit(libc::RLIMIT_CORE, &core) != 0
            || libc::prctl(
                libc::PR_SET_PDEATHSIG,
                libc::SIGKILL as libc::c_ulong,
                0,
                0,
                0,
            ) != 0
        {
            return Err(io::Error::last_os_error());
        }
        if libc::getppid() != expected_parent {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
    }
    arm_process_timer(libc::CLOCK_BOOTTIME, 120)
}

/// A one-shot timer belongs to this process only. CLOCK_BOOTTIME includes
/// suspend time but does not wake the device. SIGKILL cannot be masked.
pub(super) fn arm_process_timer(clock: libc::clockid_t, seconds: libc::time_t) -> io::Result<()> {
    if seconds <= 0 {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    // SAFETY: zero is a valid initializer for the POSIX structures below.
    // The timer is initialized by libc before use, and on successful arming
    // the kernel owns its lifetime until process exit, including SSH loss.
    unsafe {
        let mut event = std::mem::zeroed::<libc::sigevent>();
        event.sigev_notify = libc::SIGEV_SIGNAL;
        event.sigev_signo = libc::SIGKILL;
        let mut id = MaybeUninit::uninit();
        if libc::timer_create(clock, &mut event, id.as_mut_ptr()) != 0 {
            return Err(io::Error::last_os_error());
        }
        let id = id.assume_init();
        let mut interval = std::mem::zeroed::<libc::itimerspec>();
        interval.it_value.tv_sec = seconds;
        if libc::timer_settime(id, 0, &interval, std::ptr::null_mut()) != 0 {
            let error = io::Error::last_os_error();
            libc::timer_delete(id);
            return Err(error);
        }
    }
    Ok(())
}
