//! One bounded, read-only firmware session for the launcher's portrait sample.

use super::{DeviceProfile, PlatformError, native_command};
use std::{
    io::{self, Read, Write},
    os::fd::AsRawFd,
    path::Path,
    process::{Child, ChildStdin, ChildStdout, Command},
    time::{Duration, Instant},
};

const READY: &[u8; 4] = b"WQ1\n";
const MAX_REPLY: usize = 128;
const POLL: Duration = Duration::from_millis(2);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum Property {
    PowerState = 1,
    Direction = 2,
}

fn failure(message: &'static str) -> PlatformError {
    PlatformError::Operation(message.into())
}

fn check_deadline(deadline: Instant) -> Result<(), PlatformError> {
    if Instant::now() >= deadline {
        Err(failure("The native calibration transaction expired"))
    } else {
        Ok(())
    }
}

fn pause(deadline: Instant) {
    std::thread::sleep(deadline.saturating_duration_since(Instant::now()).min(POLL));
}

fn read_exact(
    output: &mut impl Read,
    buffer: &mut [u8],
    deadline: Instant,
) -> Result<(), PlatformError> {
    let mut filled = 0;
    while filled < buffer.len() {
        check_deadline(deadline)?;
        match output.read(&mut buffer[filled..]) {
            Ok(0) => return Err(failure("The native query reply ended early")),
            Ok(count) => filled += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => pause(deadline),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

struct Session {
    input: Option<ChildStdin>,
    output: ChildStdout,
    deadline: Instant,
    requests: usize,
}

impl Session {
    fn from_child(child: &mut Child, deadline: Instant) -> Result<Self, PlatformError> {
        let input = child
            .stdin
            .take()
            .ok_or_else(|| failure("Native query stdin is absent"))?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| failure("Native query stdout is absent"))?;
        native_command::set_nonblocking(input.as_raw_fd())?;
        native_command::set_nonblocking(output.as_raw_fd())?;
        Ok(Self {
            input: Some(input),
            output,
            deadline,
            requests: 0,
        })
    }

    fn handshake(&mut self) -> bool {
        let mut bytes = [0; 4];
        read_exact(&mut self.output, &mut bytes, self.deadline).is_ok() && &bytes == READY
    }

    fn query(&mut self, property: Property) -> Result<String, PlatformError> {
        if self.requests >= 4 {
            return Err(failure("The calibration query allowance was exceeded"));
        }
        self.requests += 1;
        let request = [property as u8];
        loop {
            check_deadline(self.deadline)?;
            match self
                .input
                .as_mut()
                .ok_or_else(|| failure("Native query stdin is closed"))?
                .write(&request)
            {
                Ok(1) => break,
                Ok(_) => return Err(failure("The native query request was not written")),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => pause(self.deadline),
                Err(error) => return Err(error.into()),
            }
        }
        let mut header = [0; 4];
        read_exact(&mut self.output, &mut header, self.deadline)?;
        let length = usize::from(u16::from_be_bytes([header[2], header[3]]));
        if header[0] != property as u8 || header[1] != 0 || length > MAX_REPLY {
            return Err(failure("The native query reply was rejected or malformed"));
        }
        let mut bytes = [0; MAX_REPLY];
        read_exact(&mut self.output, &mut bytes[..length], self.deadline)?;
        std::str::from_utf8(&bytes[..length])
            .map(str::to_owned)
            .map_err(|_| failure("The native query reply is not UTF-8"))
    }

    fn finish(mut self, child: &mut Child) -> Result<(), PlatformError> {
        drop(self.input.take());
        let mut eof = false;
        let mut status = None;
        loop {
            check_deadline(self.deadline)?;
            if !eof {
                let mut extra = [0];
                match self.output.read(&mut extra) {
                    Ok(0) => eof = true,
                    Ok(_) => {
                        return Err(failure("The native query session returned trailing data"));
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                    Err(error) => return Err(error.into()),
                }
            }
            if status.is_none() {
                status = child.try_wait()?;
            }
            if let Some(status) = status {
                if !status.success() {
                    return Err(failure(
                        "The native query session did not exit successfully",
                    ));
                }
                if eof {
                    return Ok(());
                }
            }
            pause(self.deadline);
        }
    }
}

fn direction(value: &str, profile: DeviceProfile) -> Result<&'static str, PlatformError> {
    match value {
        "U" => Ok("U"),
        "D" => Ok("D"),
        "L" if profile == DeviceProfile::Scribe => Ok("U"),
        "R" if profile == DeviceProfile::Scribe => Ok("D"),
        _ => Err(failure("The native portrait direction is unknown")),
    }
}

fn calibrate(
    profile: DeviceProfile,
    mut query: impl FnMut(Property) -> Result<String, PlatformError>,
    mut framebuffer: impl FnMut() -> Result<(bool, u32), PlatformError>,
) -> Result<String, PlatformError> {
    if query(Property::PowerState)? != "active" {
        return Err(failure("The native power state is not active"));
    }
    let before = framebuffer()?;
    if before.1 > 3 {
        return Err(failure("The native framebuffer rotation is invalid"));
    }
    let first = if before.0 {
        direction(&query(Property::Direction)?, profile)?
    } else {
        "FIXED"
    };
    if framebuffer()? != before {
        return Err(failure(
            "The native framebuffer metadata changed during calibration",
        ));
    }
    if before.0 && direction(&query(Property::Direction)?, profile)? != first {
        return Err(failure(
            "The native portrait direction changed during calibration",
        ));
    }
    if query(Property::PowerState)? != "active" {
        return Err(failure("The native power state changed during calibration"));
    }
    Ok(format!("{first}:{}", before.1))
}

/// None means the helper never completed its availability handshake. Connected
/// failures are errors and must not be mixed with a subsequent legacy sample.
pub fn query_native_portrait_calibration(
    helper: &Path,
    profile: DeviceProfile,
    framebuffer: impl FnMut() -> Result<(bool, u32), PlatformError>,
) -> Result<Option<String>, PlatformError> {
    let mut connected = false;
    let result = native_command::run_transaction(
        Command::new(helper).arg("--session"),
        |child, deadline| {
            let mut session = Session::from_child(child, deadline)?;
            if !session.handshake() {
                return Ok(None);
            }
            connected = true;
            let result = calibrate(profile, |property| session.query(property), framebuffer)?;
            check_deadline(deadline)?;
            session.finish(child)?;
            Ok(Some(result))
        },
    );
    match result {
        Err(PlatformError::Io(error))
            if !connected
                && matches!(
                    error.raw_os_error(),
                    Some(
                        libc::ENOENT | libc::EACCES | libc::ENOEXEC | libc::ELIBBAD | libc::ELIBACC
                    )
                ) =>
        {
            Ok(None)
        }
        other => other,
    }
}

#[cfg(test)]
#[path = "native_calibration_tests.rs"]
mod tests;
