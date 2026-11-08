//! Observe the dedicated portrait sensor without taking input ownership.
//!
//! Only a named pressure-only accelerometer is eligible. This descriptor never
//! receives EVIOCGRAB, and no touchscreen event stream is consumed here.

use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::mem::size_of;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};

const EVENT_SYNC: u16 = 0;
const EVENT_KEY: u8 = 1;
const EVENT_RELATIVE: u8 = 2;
const EVENT_ABSOLUTE: u16 = 3;
const SYNC_REPORT: u16 = 0;
const SYNC_DROPPED: u16 = 3;
const PRESSURE_AXIS: u16 = 0x18;
const NAME_BYTES: usize = 128;
const WORD_BITS: usize = size_of::<libc::c_ulong>() * 8;
const ABS_WORDS: usize = 64 / WORD_BITS;
const MAX_EVENTS_PER_POLL: usize = 128;
const MAX_READ_ATTEMPTS: usize = 256;

type EventBits = [libc::c_ulong; 1];
type AbsoluteBits = [libc::c_ulong; ABS_WORDS];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NativePortrait {
    Up,
    Down,
}

impl NativePortrait {
    pub(crate) const fn offset(self) -> u32 {
        match self {
            Self::Up => 0,
            Self::Down => 180,
        }
    }
}

fn decode_portrait(value: i32) -> Option<NativePortrait> {
    match value {
        // The reviewed Scribe contract groups U/L and D/R into portrait poses.
        15 | 17 => Some(NativePortrait::Up),
        16 | 18 => Some(NativePortrait::Down),
        _ => None,
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct InputEvent {
    _seconds: libc::c_long,
    _microseconds: libc::c_long,
    kind: u16,
    code: u16,
    value: i32,
}

const EVENT_BYTES: usize = size_of::<InputEvent>();

#[derive(Default)]
struct DirectionState {
    current: Option<NativePortrait>,
    pending: Option<NativePortrait>,
    dropped: bool,
    ready_sample: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SyncAction {
    Continue,
    Resynchronize,
}

impl DirectionState {
    fn accept(&mut self, event: InputEvent) -> SyncAction {
        match (event.kind, event.code) {
            (EVENT_SYNC, SYNC_DROPPED) => {
                self.pending = None;
                self.dropped = true;
                self.ready_sample = false;
            }
            (EVENT_SYNC, SYNC_REPORT) if self.dropped => {
                return SyncAction::Resynchronize;
            }
            (EVENT_SYNC, SYNC_REPORT) => {
                if let Some(pose) = self.pending.take() {
                    self.current = Some(pose);
                    self.ready_sample = true;
                }
            }
            (EVENT_ABSOLUTE, PRESSURE_AXIS) if !self.dropped => {
                // An invalid final value does not turn a partial report into a
                // direction change. Keep the last complete valid orientation.
                self.pending = decode_portrait(event.value);
            }
            _ => {}
        }
        SyncAction::Continue
    }

    fn resynchronize(&mut self, current: Option<NativePortrait>) {
        self.pending = None;
        self.dropped = false;
        if current.is_some() {
            self.current = current;
        }
        self.ready_sample = current.is_some();
    }

    fn is_synchronized(&self) -> bool {
        !self.dropped && self.pending.is_none()
    }

    fn take_sample(&mut self) -> Option<NativePortrait> {
        if self.ready_sample {
            self.ready_sample = false;
            self.current
        } else {
            None
        }
    }
}

enum ReadOutcome {
    Event(InputEvent),
    Empty,
    Budget,
}

fn read_event(
    reader: &mut impl Read,
    bytes: &mut [u8; EVENT_BYTES],
    filled: &mut usize,
    attempts: &mut usize,
) -> io::Result<ReadOutcome> {
    loop {
        if *attempts == MAX_READ_ATTEMPTS {
            return Ok(ReadOutcome::Budget);
        }
        *attempts += 1;
        match reader.read(&mut bytes[*filled..]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "portrait sensor event stream ended",
                ));
            }
            Ok(count) => {
                *filled += count;
                if *filled == EVENT_BYTES {
                    // SAFETY: Every byte of this integer-only repr(C) value has
                    // been initialized. read_unaligned does not require the
                    // byte buffer to have the input_event struct's alignment.
                    let event = unsafe {
                        std::ptr::read_unaligned(bytes.as_ptr().cast::<InputEvent>())
                    };
                    *filled = 0;
                    return Ok(ReadOutcome::Event(event));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                return Ok(ReadOutcome::Empty);
            }
            Err(error) => return Err(error),
        }
    }
}

const fn read_request(number: u8, bytes: usize) -> libc::c_ulong {
    (2 << 30)
        | ((bytes as libc::c_ulong) << 16)
        | ((b'E' as libc::c_ulong) << 8)
        | number as libc::c_ulong
}

fn has_bit(bits: &[libc::c_ulong], bit: usize) -> bool {
    bits.get(bit / WORD_BITS)
        .is_some_and(|word| *word & ((1 as libc::c_ulong) << (bit % WORD_BITS)) != 0)
}

fn pressure_only(axes: &AbsoluteBits) -> bool {
    axes.iter().enumerate().all(|(index, word)| {
        let expected = if index == PRESSURE_AXIS as usize / WORD_BITS {
            (1 as libc::c_ulong) << (PRESSURE_AXIS as usize % WORD_BITS)
        } else {
            0
        };
        *word == expected
    })
}

fn eligible_events(events: &EventBits, axes: &AbsoluteBits) -> bool {
    has_bit(events, EVENT_ABSOLUTE as usize)
        && !has_bit(events, EVENT_KEY as usize)
        && !has_bit(events, EVENT_RELATIVE as usize)
        && pressure_only(axes)
}

fn trusted_name(name: &[u8]) -> bool {
    let end = name.iter().position(|byte| *byte == 0).unwrap_or(name.len());
    let name = &name[..end];
    name.eq_ignore_ascii_case(b"bma4xy_feature")
        || name
            .split(|byte| !byte.is_ascii_alphanumeric())
            .any(|part| part.eq_ignore_ascii_case(b"kx132"))
}

fn query_bits<const N: usize>(file: &File, event: u8) -> io::Result<[libc::c_ulong; N]> {
    let mut bits = [0; N];
    let request = read_request(0x20 + event, size_of::<[libc::c_ulong; N]>());
    // SAFETY: The initialized native-word bitmap has exactly the length encoded
    // in this metadata-only ioctl. The kernel does not retain the pointer.
    let result = unsafe { libc::ioctl(file.as_raw_fd(), request as _, bits.as_mut_ptr()) };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(bits)
    }
}

fn inspect_sensor(file: &File) -> io::Result<bool> {
    if !file.metadata()?.file_type().is_char_device() {
        return Ok(false);
    }
    let events = query_bits::<1>(file, 0)?;
    if !has_bit(&events, EVENT_ABSOLUTE as usize)
        || has_bit(&events, EVENT_KEY as usize)
        || has_bit(&events, EVENT_RELATIVE as usize)
    {
        return Ok(false);
    }
    let axes = query_bits::<ABS_WORDS>(file, EVENT_ABSOLUTE as u8)?;
    if !eligible_events(&events, &axes) {
        return Ok(false);
    }
    let mut name = [0u8; NAME_BYTES];
    // SAFETY: name has the length encoded in EVIOCGNAME and is live for the
    // whole read-only call. Parsing stops at the first NUL within this buffer.
    let result = unsafe {
        libc::ioctl(
            file.as_raw_fd(),
            read_request(0x06, NAME_BYTES) as _,
            name.as_mut_ptr(),
        )
    };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    name[NAME_BYTES - 1] = 0;
    Ok(trusted_name(&name))
}

fn current_pose(file: &File) -> io::Result<Option<NativePortrait>> {
    let mut information = [0i32; 6];
    // SAFETY: input_absinfo is six aligned i32 fields. The request encodes this
    // exact 24-byte size, and the kernel does not retain the writable pointer.
    let result = unsafe {
        libc::ioctl(
            file.as_raw_fd(),
            read_request(0x40 + PRESSURE_AXIS as u8, size_of::<[i32; 6]>()) as _,
            information.as_mut_ptr(),
        )
    };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(decode_portrait(information[0]))
    }
}

/// Inspect the sensor identity and capabilities without reading its events.
/// The temporary descriptor is closed before returning to the launcher probe.
pub(crate) fn has_trusted_portrait_sensor() -> io::Result<bool> {
    Ok(GyroObserver::open()?.is_some())
}

/// Query only fresh EVIOCGABS metadata from the unique validated sensor.
/// Unlike the event-loop snapshot, this does not drain or consume any events.
pub(crate) fn read_current_portrait_pose() -> io::Result<Option<NativePortrait>> {
    let Some(observer) = GyroObserver::open()? else {
        return Ok(None);
    };
    current_pose(&observer.file)
}

pub(crate) struct GyroObserver {
    file: File,
    state: DirectionState,
    bytes: [u8; EVENT_BYTES],
    filled: usize,
    backlog: bool,
}

impl GyroObserver {
    /// Retain exactly one recognized, pressure-only sensor. All descriptors of
    /// rejected candidates close immediately through File's ordinary Drop.
    pub(crate) fn open() -> io::Result<Option<Self>> {
        let mut found = None;
        for index in 0..=31 {
            let path = format!("/dev/input/event{index}");
            let file = match OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC | libc::O_NOFOLLOW)
                .open(&path)
            {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            if !inspect_sensor(&file)? {
                continue;
            }
            if found.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "multiple recognized portrait sensors are ambiguous",
                ));
            }
            found = Some(Self {
                file,
                state: DirectionState::default(),
                bytes: [0; EVENT_BYTES],
                filled: 0,
                backlog: false,
            });
        }
        Ok(found)
    }

    pub(crate) fn fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }

    /// The caller must defer touch while an older direction report is queued.
    pub(crate) fn has_backlog(&self) -> bool {
        self.backlog || self.filled != 0
    }

    pub(crate) fn is_synchronized(&self) -> bool {
        self.state.is_synchronized() && self.filled == 0
    }

    /// Discard an old queue before using a fresh current-value metadata sample.
    /// A saturated queue returns WouldBlock for the next readiness iteration;
    /// no cached orientation is published as a successful fresh sample.
    pub(crate) fn snapshot(&mut self) -> io::Result<Option<NativePortrait>> {
        let mut attempts = 0;
        for _ in 0..MAX_EVENTS_PER_POLL {
            match read_event(
                &mut self.file,
                &mut self.bytes,
                &mut self.filled,
                &mut attempts,
            )? {
                ReadOutcome::Event(_) => {}
                ReadOutcome::Budget => {
                    self.backlog = true;
                    return Err(snapshot_backlog());
                }
                ReadOutcome::Empty if self.filled != 0 => {
                    self.backlog = true;
                    return Err(snapshot_backlog());
                }
                ReadOutcome::Empty => {
                    let pose = current_pose(&self.file)?;
                    self.state.resynchronize(pose);
                    self.state.ready_sample = false;
                    self.backlog = false;
                    return Ok(pose);
                }
            }
        }
        self.backlog = true;
        Err(snapshot_backlog())
    }

    /// Coalesce complete direction reports within a finite read budget. The
    /// newest complete valid sample is published after reaching EAGAIN, even
    /// when its pose repeats: the native scanout may have changed independently.
    pub(crate) fn poll(&mut self) -> io::Result<Option<NativePortrait>> {
        let mut attempts = 0;
        for _ in 0..MAX_EVENTS_PER_POLL {
            match read_event(
                &mut self.file,
                &mut self.bytes,
                &mut self.filled,
                &mut attempts,
            )? {
                ReadOutcome::Event(event) => {
                    if self.state.accept(event) == SyncAction::Resynchronize {
                        self.state.resynchronize(current_pose(&self.file)?);
                    }
                }
                ReadOutcome::Budget => {
                    self.backlog = true;
                    return Ok(None);
                }
                ReadOutcome::Empty => {
                    self.backlog = self.filled != 0;
                    if !self.is_synchronized() || self.has_backlog() {
                        return Ok(None);
                    }
                    return Ok(self.state.take_sample());
                }
            }
        }
        self.backlog = true;
        Ok(None)
    }
}

fn snapshot_backlog() -> io::Error {
    io::Error::new(
        io::ErrorKind::WouldBlock,
        "portrait snapshot requires an empty synchronized event queue",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: u16, code: u16, value: i32) -> InputEvent {
        InputEvent {
            _seconds: 0,
            _microseconds: 0,
            kind,
            code,
            value,
        }
    }

    fn report(state: &mut DirectionState, value: i32) -> SyncAction {
        state.accept(event(EVENT_ABSOLUTE, PRESSURE_AXIS, value));
        state.accept(event(EVENT_SYNC, SYNC_REPORT, 0))
    }

    #[test]
    fn portrait_contract_ignores_invalid_and_flat_values() {
        for value in [15, 17] {
            assert_eq!(decode_portrait(value), Some(NativePortrait::Up));
        }
        for value in [16, 18] {
            assert_eq!(decode_portrait(value), Some(NativePortrait::Down));
        }
        for value in [i32::MIN, -1, 0, 1, 14, 19, i32::MAX] {
            assert_eq!(decode_portrait(value), None);
        }
        assert_eq!(NativePortrait::Up.offset(), 0);
        assert_eq!(NativePortrait::Down.offset(), 180);
    }

    #[test]
    fn direction_changes_commit_only_at_a_complete_report() {
        let mut state = DirectionState::default();
        assert_eq!(report(&mut state, 15), SyncAction::Continue);
        state.accept(event(EVENT_ABSOLUTE, PRESSURE_AXIS, 16));
        assert_eq!(state.current, Some(NativePortrait::Up));
        assert!(!state.is_synchronized());
        state.accept(event(EVENT_SYNC, SYNC_REPORT, 0));
        assert_eq!(state.current, Some(NativePortrait::Down));
        assert!(state.is_synchronized());
        report(&mut state, 0);
        assert_eq!(state.current, Some(NativePortrait::Down));
        state.accept(event(EVENT_ABSOLUTE, PRESSURE_AXIS, 15));
        state.accept(event(EVENT_ABSOLUTE, PRESSURE_AXIS, 0));
        state.accept(event(EVENT_SYNC, SYNC_REPORT, 0));
        assert_eq!(state.current, Some(NativePortrait::Down));
    }

    #[test]
    fn complete_repeated_pose_reports_request_one_fresh_calibration_each() {
        let mut state = DirectionState::default();
        report(&mut state, 15);
        assert_eq!(state.take_sample(), Some(NativePortrait::Up));
        assert_eq!(state.take_sample(), None);
        report(&mut state, 15);
        assert_eq!(state.take_sample(), Some(NativePortrait::Up));
        assert_eq!(state.take_sample(), None);
        report(&mut state, 0);
        assert_eq!(state.take_sample(), None);
        state.accept(event(EVENT_SYNC, SYNC_DROPPED, 0));
        assert_eq!(state.take_sample(), None);
        state.resynchronize(Some(NativePortrait::Up));
        assert_eq!(state.take_sample(), Some(NativePortrait::Up));
        assert_eq!(state.take_sample(), None);
        state.resynchronize(None);
        assert_eq!(state.take_sample(), None);
    }

    #[test]
    fn dropped_reports_require_a_fresh_snapshot_before_publication() {
        let mut state = DirectionState::default();
        report(&mut state, 15);
        state.accept(event(EVENT_ABSOLUTE, PRESSURE_AXIS, 16));
        state.accept(event(EVENT_SYNC, SYNC_DROPPED, 0));
        state.accept(event(EVENT_ABSOLUTE, PRESSURE_AXIS, 18));
        assert_eq!(state.current, Some(NativePortrait::Up));
        assert_eq!(
            state.accept(event(EVENT_SYNC, SYNC_REPORT, 0)),
            SyncAction::Resynchronize
        );
        assert!(state.dropped);
        state.resynchronize(Some(NativePortrait::Down));
        assert!(!state.dropped);
        assert_eq!(state.current, Some(NativePortrait::Down));
        state.resynchronize(None);
        assert_eq!(state.current, Some(NativePortrait::Down));
    }

    #[test]
    fn sensor_names_are_bounded_and_keep_the_reviewed_identity() {
        for name in [
            &b"kx132"[..],
            &b"KX132 accelerometer"[..],
            &b"kx132-1211"[..],
            &b"bma4xy_feature\0ignored"[..],
        ] {
            assert!(trusted_name(name));
        }
        for name in [
            &b""[..],
            &b"unknownkx132touch"[..],
            &b"kx134"[..],
            &b"other bma4xy_feature"[..],
            &b"touch\0kx132"[..],
        ] {
            assert!(!trusted_name(name));
        }
    }

    #[test]
    fn pressure_only_capabilities_reject_touch_keys_and_relative_axes() {
        let mut events = [0; 1];
        let mut axes = [0; ABS_WORDS];
        events[0] = (1 as libc::c_ulong) << EVENT_ABSOLUTE;
        axes[PRESSURE_AXIS as usize / WORD_BITS] =
            (1 as libc::c_ulong) << (PRESSURE_AXIS as usize % WORD_BITS);
        assert!(eligible_events(&events, &axes));
        for extra in 0..64 {
            if extra != PRESSURE_AXIS as usize {
                let mut extra_axes = axes;
                extra_axes[extra / WORD_BITS] |= (1 as libc::c_ulong) << (extra % WORD_BITS);
                assert!(!eligible_events(&events, &extra_axes));
            }
        }
        for kind in [EVENT_KEY, EVENT_RELATIVE] {
            let mut extra_events = events;
            extra_events[0] |= (1 as libc::c_ulong) << kind;
            assert!(!eligible_events(&extra_events, &axes));
        }
    }

    #[test]
    fn partial_reads_preserve_native_event_fields() {
        struct Chunks {
            bytes: [u8; EVENT_BYTES],
            position: usize,
        }
        impl Read for Chunks {
            fn read(&mut self, destination: &mut [u8]) -> io::Result<usize> {
                let count = destination.len().min(3).min(EVENT_BYTES - self.position);
                destination[..count]
                    .copy_from_slice(&self.bytes[self.position..self.position + count]);
                self.position += count;
                Ok(count)
            }
        }
        let mut bytes = [0u8; EVENT_BYTES];
        let offset = 2 * size_of::<libc::c_long>();
        bytes[offset..offset + 2].copy_from_slice(&EVENT_ABSOLUTE.to_ne_bytes());
        bytes[offset + 2..offset + 4].copy_from_slice(&PRESSURE_AXIS.to_ne_bytes());
        bytes[offset + 4..offset + 8].copy_from_slice(&16i32.to_ne_bytes());
        let mut reader = Chunks { bytes, position: 0 };
        let mut received = [0; EVENT_BYTES];
        let mut filled = 0;
        let mut attempts = 0;
        let ReadOutcome::Event(event) =
            read_event(&mut reader, &mut received, &mut filled, &mut attempts).unwrap()
        else {
            panic!("A complete native event was expected");
        };
        assert_eq!((event.kind, event.code, event.value), (EVENT_ABSOLUTE, PRESSURE_AXIS, 16));
        assert_eq!(filled, 0);
    }

    #[test]
    fn an_interrupted_reader_cannot_starve_the_control_loop() {
        struct Interrupts(usize);
        impl Read for Interrupts {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                self.0 += 1;
                Err(io::Error::from(io::ErrorKind::Interrupted))
            }
        }
        let mut reader = Interrupts(0);
        let mut bytes = [0; EVENT_BYTES];
        let mut filled = 0;
        let mut attempts = 0;
        assert!(matches!(
            read_event(&mut reader, &mut bytes, &mut filled, &mut attempts).unwrap(),
            ReadOutcome::Budget
        ));
        assert_eq!(reader.0, MAX_READ_ATTEMPTS);
        assert_eq!(attempts, MAX_READ_ATTEMPTS);
        assert_eq!(filled, 0);
    }
}
