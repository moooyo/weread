use slint::LogicalPosition;
use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::platform::{PointerEventButton, WindowEvent};

// Touchscreen driver event. Layout has to match exactly what the kernel writes.
#[repr(C)]
struct TouchInputEvent {
    timestamp_seconds: libc::c_long,
    timestamp_microseconds: libc::c_long,
    kind: u16,
    code: u16,
    value: i32,
}

// Kernel-defined IDs we match against the `kind` and `code` fields of each event.
const EVENT_SYNC: u16 = 0;
const EVENT_ABSOLUTE_AXIS: u16 = 3;
const EVENT_KEY: u16 = 1; //Amazon "snow protocol", BTN_TOUCH required instead of ABS_MT_TRACKING_ID for some KT4 panels.
const SYNC_REPORT: u16 = 0;
const SYNC_DROPPED: u16 = 3;
const SINGLE_TOUCH_POSITION_X: u16 = 0x00;
const PRESSURE_AXIS: u16 = 0x18;
const TOUCH_SLOT: u16 = 0x2f;
const TOUCH_POSITION_X: u16 = 0x35;
const TOUCH_POSITION_Y: u16 = 0x36;
const TOUCH_TRACKING_ID: u16 = 0x39;
const BTN_TOUCH: u16 = 0x14a;
const MAX_TRANSITION_DRAIN: usize = 256;
const MAX_EVENTS_PER_POLL: usize = 256;
const MAX_TOUCH_SLOTS: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TransitionDrain {
    Ready,
    Waiting,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum DrainSynchronization {
    #[default]
    Complete,
    Dropped,
    SnapshotPending,
}

// The following constants/functions replicate Linux kernel C macros from <linux/input.h>
// that aren't exposed by the `libc` crate. The formulas expand the _IOW/_IOR macro:
//   _IOC(dir, type, nr, size) = (dir << 30) | (size << 16) | (type << 8) | nr

// EVIOCGRAB: exclusively grabs an input device so no other process receives
// its events. This is essential on Kindle Touch because the zforce touch controller
// is initialized by the framework/X — if we stop the framework, zforce disconnects
// and becomes unusable. Instead, we keep the framework running and grab the device.
// C macro: #define EVIOCGRAB _IOW('E', 0x90, int)
const EVIOCGRAB: libc::c_ulong = 0x40044590;

// C macro: #define EVIOCGABS(abs) _IOR('E', 0x40 + (abs), struct input_absinfo)
// struct input_absinfo is 6 × i32 = 24 bytes
fn eviocgabs(axis: u8) -> libc::c_ulong {
    (2 << 30) | (24 << 16) | ((b'E' as libc::c_ulong) << 8) | (0x40 + axis as libc::c_ulong)
}

/// Kernel struct returned by EVIOCGABS.
#[repr(C)]
#[derive(Default)]
struct InputAbsinfo {
    value: i32,
    minimum: i32,
    maximum: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}

struct ContactSnapshot {
    active_slot: i32,
    tracking_ids: [i32; MAX_TOUCH_SLOTS],
    legacy_held: bool,
    uses_tracking_ids: bool,
    coordinates: [SlotCoordinates; MAX_TOUCH_SLOTS],
}

#[derive(Clone, Copy)]
struct SlotCoordinates {
    x: Option<i32>,
    y: Option<i32>,
}

impl SlotCoordinates {
    const EMPTY: Self = Self { x: None, y: None };

    fn complete(self) -> Option<(i32, i32)> {
        Some((self.x?, self.y?))
    }
}

fn read_ioctl(number: u8, bytes: usize) -> libc::c_ulong {
    (2 << 30)
        | ((bytes as libc::c_ulong) << 16)
        | ((b'E' as libc::c_ulong) << 8)
        | number as libc::c_ulong
}

fn query_contact_snapshot(fd: libc::c_int) -> std::io::Result<ContactSnapshot> {
    let mut axes = [0u8; 8];
    // SAFETY: The initialized bitmap has the length encoded in EVIOCGBIT.
    let result = unsafe { libc::ioctl(fd, read_ioctl(0x23, axes.len()) as _, axes.as_mut_ptr()) };
    if result < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let has_axis = |axis: u16| axes[axis as usize / 8] & (1 << (axis % 8)) != 0;
    if has_axis(TOUCH_TRACKING_ID) {
        if !has_axis(TOUCH_SLOT) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "loss recovery requires protocol-B tracking slots",
            ));
        }
        let mut slot = InputAbsinfo::default();
        // SAFETY: slot has the complete input_absinfo layout encoded by EVIOCGABS.
        if unsafe { libc::ioctl(fd, eviocgabs(TOUCH_SLOT as u8) as _, &mut slot) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if slot.minimum != 0
            || slot.maximum < 0
            || slot.maximum >= MAX_TOUCH_SLOTS as i32
            || slot.value < 0
            || slot.value > slot.maximum
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid protocol-B touch slot range",
            ));
        }
        let count = slot.maximum as usize + 1;
        let tracking_ids = query_slot_values(fd, TOUCH_TRACKING_ID, count)?;
        if !has_axis(TOUCH_POSITION_X) || !has_axis(TOUCH_POSITION_Y) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "touch loss recovery requires current slot coordinates",
            ));
        }
        let x = query_slot_values(fd, TOUCH_POSITION_X, count)?;
        let y = query_slot_values(fd, TOUCH_POSITION_Y, count)?;
        let mut coordinates = [SlotCoordinates::EMPTY; MAX_TOUCH_SLOTS];
        for slot in 0..count {
            coordinates[slot] = SlotCoordinates {
                x: Some(x[slot]),
                y: Some(y[slot]),
            };
        }
        if tracking_ids.iter().any(|id| *id < -1) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid current touch tracking ID",
            ));
        }
        return Ok(ContactSnapshot {
            active_slot: slot.value,
            tracking_ids,
            legacy_held: false,
            uses_tracking_ids: true,
            coordinates,
        });
    }

    let mut key_bits = [0u8; 64];
    // SAFETY: The initialized bitmap covers BTN_TOUCH and matches the ioctl size.
    if unsafe {
        libc::ioctl(
            fd,
            read_ioctl(0x21, key_bits.len()) as _,
            key_bits.as_mut_ptr(),
        )
    } < 0
    {
        return Err(std::io::Error::last_os_error());
    }
    let touch_bit = |bits: &[u8]| bits[BTN_TOUCH as usize / 8] & (1 << (BTN_TOUCH % 8)) != 0;
    if !touch_bit(&key_bits) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "touch loss recovery has no tracking IDs or BTN_TOUCH snapshot",
        ));
    }
    let mut keys = [0u8; 64];
    // SAFETY: EVIOCGKEY is metadata-only and uses the live initialized bitmap.
    if unsafe { libc::ioctl(fd, read_ioctl(0x18, keys.len()) as _, keys.as_mut_ptr()) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let x_axis = if has_axis(TOUCH_POSITION_X) {
        TOUCH_POSITION_X
    } else {
        SINGLE_TOUCH_POSITION_X
    };
    let y_axis = if has_axis(TOUCH_POSITION_Y) {
        TOUCH_POSITION_Y
    } else {
        0x01
    };
    if !has_axis(x_axis) || !has_axis(y_axis) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "legacy touch loss recovery has no current coordinates",
        ));
    }
    let mut coordinates = [SlotCoordinates::EMPTY; MAX_TOUCH_SLOTS];
    coordinates[0] = SlotCoordinates {
        x: Some(query_axis_value(fd, x_axis)?),
        y: Some(query_axis_value(fd, y_axis)?),
    };
    Ok(ContactSnapshot {
        active_slot: 0,
        tracking_ids: [-1; MAX_TOUCH_SLOTS],
        legacy_held: touch_bit(&keys),
        uses_tracking_ids: false,
        coordinates,
    })
}

fn query_slot_values(
    fd: libc::c_int,
    axis: u16,
    count: usize,
) -> std::io::Result<[i32; MAX_TOUCH_SLOTS]> {
    let mut values = [-1i32; MAX_TOUCH_SLOTS + 1];
    values[0] = axis as i32;
    // SAFETY: EVIOCGMTSLOTS writes count values following the requested axis
    // code. The bounded buffer has the exact requested capacity.
    let result = unsafe {
        libc::ioctl(
            fd,
            read_ioctl(0x0a, (count + 1) * std::mem::size_of::<i32>()) as _,
            values.as_mut_ptr(),
        )
    };
    if result < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut result = [-1; MAX_TOUCH_SLOTS];
    result[..count].copy_from_slice(&values[1..=count]);
    Ok(result)
}

fn query_axis_value(fd: libc::c_int, axis: u16) -> std::io::Result<i32> {
    let mut information = InputAbsinfo::default();
    // SAFETY: The initialized input_absinfo has the complete encoded ABI layout.
    if unsafe { libc::ioctl(fd, eviocgabs(axis as u8) as _, &mut information) } < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(information.value)
    }
}

pub(crate) struct TouchInput {
    file_descriptor: libc::c_int,
    // Protocol-B slot that the incoming ABS_MT_* events currently describe. The
    // driver only emits ABS_MT_SLOT once a second finger appears, so the first
    // contact is implicitly slot 0.
    active_slot: i32,
    // The slot we treat as "the pointer". Slint has no concept of more than one
    // pointer, so we lock onto the first finger down and ignore the others until
    // it lifts. Without this, a second finger's coordinates land in the same
    // x/y and the pointer jitters back and forth between the two fingers.
    tracked_slot: Option<i32>,
    x: f32,
    y: f32,
    pressed: bool,
    screen_width: f32,
    screen_height: f32,
    /// Rotation the panel is currently drawn with, handed over on each poll.
    rotation: u32,
    /// Maximum raw X value reported by the touch controller (from EVIOCGABS).
    max_x: f32,
    /// Maximum raw Y value reported by the touch controller (from EVIOCGABS).
    max_y: f32,
    /// A cancelled contact's queued events cannot cross a presentation change.
    draining: bool,
    drain_synchronization: DrainSynchronization,
    current_tracking: [i32; MAX_TOUCH_SLOTS],
    cancelled_tracking: [i32; MAX_TOUCH_SLOTS],
    legacy_held: bool,
    cancelled_legacy: bool,
    uses_tracking_ids: bool,
    loss_coordinates: Option<[SlotCoordinates; MAX_TOUCH_SLOTS]>,
}

impl TouchInput {
    pub(crate) fn open(screen_width: u32, screen_height: u32) -> std::io::Result<Self> {
        let path = Self::find_touch_device().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no touch input device found (looked for ABS_MT_POSITION_X in /dev/input/event*)",
            )
        })?;
        let c_path = std::ffi::CString::new(path).unwrap();
        let fd = unsafe {
            libc::open(
                c_path.as_ptr(),
                libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }

        // Exclusively grab the device so X/framework doesn't consume our events.
        let ret = unsafe { libc::ioctl(fd, EVIOCGRAB as _, 1 as libc::c_int) };
        if ret < 0 {
            let err = std::io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(err);
        }

        // Query the axis ranges from the device so we scale correctly. On
        // failure we fall back to a common 12-bit max, but log it: a wrong range
        // silently maps touches to the wrong coordinates, which is otherwise
        // baffling to diagnose.
        let max_x = Self::query_axis_max(fd, TOUCH_POSITION_X as u8).unwrap_or_else(|| {
            log::warn!(
                "could not read touch X axis range; assuming max 4095, touch may be miscalibrated"
            );
            4095
        }) as f32;
        let max_y = Self::query_axis_max(fd, TOUCH_POSITION_Y as u8).unwrap_or_else(|| {
            log::warn!(
                "could not read touch Y axis range; assuming max 4095, touch may be miscalibrated"
            );
            4095
        }) as f32;

        Ok(Self {
            file_descriptor: fd,
            active_slot: 0,
            tracked_slot: None,
            x: 0.0,
            y: 0.0,
            pressed: false,
            screen_width: screen_width as f32,
            screen_height: screen_height as f32,
            rotation: 0,
            max_x,
            max_y,
            draining: false,
            drain_synchronization: DrainSynchronization::Complete,
            current_tracking: [-1; MAX_TOUCH_SLOTS],
            cancelled_tracking: [-1; MAX_TOUCH_SLOTS],
            legacy_held: false,
            cancelled_legacy: false,
            uses_tracking_ids: false,
            loss_coordinates: None,
        })
    }

    /// Query the maximum value for an absolute axis via EVIOCGABS.
    fn query_axis_max(fd: libc::c_int, axis: u8) -> Option<i32> {
        let mut info = InputAbsinfo::default();
        let ret = unsafe { libc::ioctl(fd, eviocgabs(axis) as _, &mut info as *mut InputAbsinfo) };
        // A non-positive maximum would make the scale factor divide by zero
        // (producing inf/NaN coordinates), so treat it as "unknown" and let the
        // caller fall back to a sane default.
        if ret < 0 || info.maximum <= 0 {
            None
        } else {
            Some(info.maximum)
        }
    }

    /// Scan /dev/input/event* for a device that reports ABS_MT_POSITION_X (0x35).
    fn find_touch_device() -> Option<String> {
        for n in 0..10 {
            let path = format!("/dev/input/event{n}");
            let c_path = std::ffi::CString::new(path.as_str()).unwrap();
            let fd = unsafe {
                libc::open(
                    c_path.as_ptr(),
                    libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                continue;
            }
            // EVIOCGBIT(EV_ABS, size) = _IOC(_IOC_READ, 'E', 0x20 + EV_ABS, size)
            // For ABS (0x03): ioctl number is 0x80404520 + 0x03 = we compute it manually.
            // We need at least 8 bytes to cover bit 0x35 (byte index 6, bit 5).
            let mut abs_bits = [0u8; 8];
            // EVIOCGBIT(EV_ABS=3, 8) on 32-bit: _IOC(2, 'E', 0x23, 8)
            let request: libc::c_ulong = 2 << 30 | 8 << 16 | (b'E' as libc::c_ulong) << 8 | 0x23;
            let ret = unsafe { libc::ioctl(fd, request as _, abs_bits.as_mut_ptr()) };
            unsafe { libc::close(fd) };
            if ret < 0 {
                continue;
            }
            // Check bit 0x35 (ABS_MT_POSITION_X): byte 0x35/8 = 6, bit 0x35%8 = 5
            if abs_bits[6] & (1 << 5) != 0 {
                return Some(path);
            }
        }
        None
    }

    /// Raw fd of the touch device, for use with `poll(2)` in the event loop.
    pub(crate) fn fd(&self) -> libc::c_int {
        self.file_descriptor
    }

    /// Cancel an in-flight contact without synthesizing a click or a swipe.
    /// Slint's PointerExited clears the grab and emits PointerEventKind::Cancel.
    pub(crate) fn cancel(&mut self, window: &MinimalSoftwareWindow) {
        let _ = window.dispatch_event(WindowEvent::PointerExited);
        self.active_slot = 0;
        self.tracked_slot = None;
        self.pressed = false;
        self.x = 0.0;
        self.y = 0.0;
    }

    /// Invalidate the old contact and consume its queued packets in finite batches.
    /// A cancelled contact cannot resume; only a genuine new identity may start.
    pub(crate) fn cancel_and_drain(
        &mut self,
        window: &MinimalSoftwareWindow,
    ) -> std::io::Result<TransitionDrain> {
        let active_slot = self.active_slot;
        self.block_current_contacts();
        self.cancel(window);
        self.active_slot = active_slot;
        self.draining = true;
        self.drain_cancelled_events()
    }

    /// Drain again after the complete panel refresh so input queued while the
    /// previous presentation was visible cannot be replayed into the new frame.
    pub(crate) fn finish_presentation_transition(&mut self) -> std::io::Result<TransitionDrain> {
        self.block_current_contacts();
        self.draining = true;
        self.drain_cancelled_events()
    }

    pub(crate) fn continue_presentation_transition(&mut self) -> std::io::Result<TransitionDrain> {
        self.drain_cancelled_events()
    }

    fn block_current_contacts(&mut self) {
        self.cancelled_tracking = self.current_tracking;
        self.cancelled_legacy = self.legacy_held && !self.uses_tracking_ids;
    }

    fn cancelled_contact_is_held(&self) -> bool {
        self.cancelled_legacy || self.cancelled_tracking.iter().any(|id| *id >= 0)
    }

    fn drain_cancelled_events(&mut self) -> std::io::Result<TransitionDrain> {
        self.drain_cancelled_events_with(query_contact_snapshot)
    }

    fn drain_cancelled_events_with(
        &mut self,
        mut query: impl FnMut(libc::c_int) -> std::io::Result<ContactSnapshot>,
    ) -> std::io::Result<TransitionDrain> {
        if !self.draining {
            return Ok(TransitionDrain::Ready);
        }
        for _ in 0..MAX_TRANSITION_DRAIN {
            if self.drain_synchronization == DrainSynchronization::SnapshotPending {
                let snapshot = match query(self.file_descriptor) {
                    Ok(snapshot) => snapshot,
                    Err(error) if is_retryable(&error) => return Ok(TransitionDrain::Waiting),
                    Err(error) => return Err(error),
                };
                self.active_slot = snapshot.active_slot;
                self.current_tracking = snapshot.tracking_ids;
                self.uses_tracking_ids = snapshot.uses_tracking_ids;
                self.legacy_held = snapshot.legacy_held;
                self.loss_coordinates = Some(snapshot.coordinates);
                self.x = 0.0;
                self.y = 0.0;
                self.block_current_contacts();
                self.drain_synchronization = DrainSynchronization::Complete;
            }
            let event = match self.read_event() {
                Ok(Some(event)) => event,
                Ok(None) if self.drain_synchronization == DrainSynchronization::Complete => {
                    self.draining = false;
                    return Ok(TransitionDrain::Ready);
                }
                Ok(None) => return Ok(TransitionDrain::Waiting),
                Err(error) if is_retryable(&error) => return Ok(TransitionDrain::Waiting),
                Err(error) => return Err(error),
            };
            match (event.kind, event.code) {
                (EVENT_SYNC, SYNC_DROPPED) => {
                    self.drain_synchronization = DrainSynchronization::Dropped;
                    self.invalidate_loss_coordinates();
                }
                (EVENT_SYNC, SYNC_REPORT)
                    if self.drain_synchronization == DrainSynchronization::Dropped =>
                {
                    self.drain_synchronization = DrainSynchronization::SnapshotPending;
                }
                _ if self.drain_synchronization != DrainSynchronization::Complete => {}
                (EVENT_ABSOLUTE_AXIS, TOUCH_SLOT) => {
                    self.active_slot = checked_slot(event.value)?;
                }
                (EVENT_ABSOLUTE_AXIS, TOUCH_TRACKING_ID) => {
                    let slot = checked_slot(self.active_slot)? as usize;
                    let id = checked_tracking_id(event.value)?;
                    self.uses_tracking_ids = true;
                    self.legacy_held = false;
                    self.cancelled_legacy = false;
                    self.current_tracking[slot] = id;
                    self.cancelled_tracking[slot] = id;
                }
                (EVENT_ABSOLUTE_AXIS, TOUCH_POSITION_X | TOUCH_POSITION_Y) => {
                    self.record_loss_coordinate(event.code, event.value)?;
                }
                (EVENT_KEY, BTN_TOUCH) if !self.uses_tracking_ids => {
                    self.legacy_held = event.value != 0;
                    self.cancelled_legacy = self.legacy_held;
                }
                _ => {}
            }
        }
        Ok(TransitionDrain::Waiting)
    }

    /// Read any waiting touch events and forward them to the window as pointer events.
    pub(crate) fn poll(
        &mut self,
        window: &MinimalSoftwareWindow,
        rotation: u32,
    ) -> std::io::Result<()> {
        self.rotation = rotation;
        if self.draining {
            self.drain_cancelled_events()?;
            return Ok(());
        }
        for _ in 0..MAX_EVENTS_PER_POLL {
            let event = match self.read_event() {
                Ok(Some(event)) => event,
                Ok(None) => break,
                Err(error) if is_retryable(&error) => break,
                Err(error) => return Err(error),
            };
            match (event.kind, event.code) {
                (EVENT_SYNC, SYNC_DROPPED) => {
                    self.cancel_and_drain_after_loss(window)?;
                    return Ok(());
                }
                (EVENT_ABSOLUTE_AXIS, TOUCH_SLOT) => {
                    self.active_slot = checked_slot(event.value)?;
                }
                (EVENT_ABSOLUTE_AXIS, TOUCH_POSITION_X) => {
                    self.record_loss_coordinate(event.code, event.value)?;
                    if self.is_tracked_slot() {
                        self.x = (event.value as f32) * self.screen_width / self.max_x;
                    }
                }
                (EVENT_ABSOLUTE_AXIS, TOUCH_POSITION_Y) => {
                    self.record_loss_coordinate(event.code, event.value)?;
                    if self.is_tracked_slot() {
                        self.y = (event.value as f32) * self.screen_height / self.max_y;
                    }
                }
                (EVENT_ABSOLUTE_AXIS, TOUCH_TRACKING_ID) => {
                    let slot = checked_slot(self.active_slot)? as usize;
                    let id = checked_tracking_id(event.value)?;
                    self.uses_tracking_ids = true;
                    self.legacy_held = false;
                    self.cancelled_legacy = false;
                    self.current_tracking[slot] = id;
                    if id == -1 {
                        self.cancelled_tracking[slot] = -1;
                        // The finger in the active slot lifted. Only end the
                        // stroke if it was the one we were following; a lifted
                        // secondary finger must not release the pointer.
                        if self.tracked_slot == Some(self.active_slot) {
                            self.tracked_slot = None;
                            self.release(window);
                        }
                    } else {
                        // A different real tracking ID proves that the old
                        // contact in this slot ended, even if its lift was lost.
                        if self.cancelled_tracking[slot] != id {
                            self.cancelled_tracking[slot] = -1;
                        }
                        if self.cancelled_contact_is_held() {
                            self.cancelled_tracking[slot] = id;
                        } else if self.tracked_slot.is_none() {
                            self.tracked_slot = Some(self.active_slot);
                            self.restore_fresh_coordinates();
                        }
                    }
                }
                (EVENT_KEY, BTN_TOUCH) if !self.uses_tracking_ids => {
                    self.legacy_held = event.value != 0;
                    if !self.legacy_held {
                        self.cancelled_legacy = false;
                        self.tracked_slot = None;
                        self.release(window);
                    } else if !self.cancelled_contact_is_held() && self.tracked_slot.is_none() {
                        self.tracked_slot = Some(self.active_slot);
                        self.restore_fresh_coordinates();
                    }
                }
                (EVENT_SYNC, SYNC_REPORT) => self.commit(window),
                _ => {}
            }
        }
        Ok(())
    }

    fn cancel_and_drain_after_loss(
        &mut self,
        window: &MinimalSoftwareWindow,
    ) -> std::io::Result<TransitionDrain> {
        self.drain_synchronization = DrainSynchronization::Dropped;
        self.invalidate_loss_coordinates();
        self.cancel_and_drain(window)
    }

    fn invalidate_loss_coordinates(&mut self) {
        self.loss_coordinates = Some([SlotCoordinates::EMPTY; MAX_TOUCH_SLOTS]);
        self.x = 0.0;
        self.y = 0.0;
    }

    fn record_loss_coordinate(&mut self, axis: u16, value: i32) -> std::io::Result<()> {
        let slot = checked_slot(self.active_slot)? as usize;
        // Keep current raw axes before an ordinary cancellation too: a new
        // tracking ID does not force evdev to repeat unchanged coordinates.
        let coordinates = self
            .loss_coordinates
            .get_or_insert([SlotCoordinates::EMPTY; MAX_TOUCH_SLOTS]);
        match axis {
            TOUCH_POSITION_X => coordinates[slot].x = Some(value),
            TOUCH_POSITION_Y => coordinates[slot].y = Some(value),
            _ => {}
        }
        Ok(())
    }

    fn restore_fresh_coordinates(&mut self) {
        let Some(slot) = self.tracked_slot else {
            return;
        };
        let Some(coordinates) = self.loss_coordinates.as_ref() else {
            return;
        };
        self.x = 0.0;
        self.y = 0.0;
        if let Some((x, y)) = coordinates[slot as usize].complete() {
            self.x = x as f32 * self.screen_width / self.max_x;
            self.y = y as f32 * self.screen_height / self.max_y;
        }
    }

    fn read_event(&self) -> std::io::Result<Option<TouchInputEvent>> {
        let mut event = TouchInputEvent {
            timestamp_seconds: 0,
            timestamp_microseconds: 0,
            kind: 0,
            code: 0,
            value: 0,
        };
        let bytes_read = unsafe {
            libc::read(
                self.file_descriptor,
                &mut event as *mut TouchInputEvent as *mut libc::c_void,
                std::mem::size_of::<TouchInputEvent>(),
            )
        };
        if bytes_read == std::mem::size_of::<TouchInputEvent>() as isize {
            return Ok(Some(event));
        }
        if bytes_read < 0 {
            let error = std::io::Error::last_os_error();
            return if error.kind() == std::io::ErrorKind::WouldBlock {
                Ok(None)
            } else {
                Err(error)
            };
        }
        Err(std::io::Error::new(
            if bytes_read == 0 {
                std::io::ErrorKind::UnexpectedEof
            } else {
                std::io::ErrorKind::InvalidData
            },
            "touch event stream ended or returned an incomplete event",
        ))
    }

    fn release(&mut self, window: &MinimalSoftwareWindow) {
        if !self.pressed {
            return;
        }
        self.pressed = false;
        let _ = window.dispatch_event(WindowEvent::PointerReleased {
            position: self.logical_position(window),
            button: PointerEventButton::Left,
        });
    }

    /// Whether the slot the driver is currently describing is the one we follow.
    fn is_tracked_slot(&self) -> bool {
        self.tracked_slot == Some(self.active_slot)
    }

    // Called when the driver signals batch of events is finished. Dispatch
    // a press if the finger just touched down, or a move if it was already down.
    fn commit(&mut self, window: &MinimalSoftwareWindow) {
        if self.tracked_slot.is_none() || self.cancelled_contact_is_held() {
            return;
        }
        if self
            .loss_coordinates
            .as_ref()
            .and_then(|coordinates| coordinates[self.tracked_slot.unwrap() as usize].complete())
            .is_none()
        {
            return;
        }
        let position = self.logical_position(window);
        let pointer_event = if self.pressed {
            WindowEvent::PointerMoved { position }
        } else {
            self.pressed = true;
            WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            }
        };
        let _ = window.dispatch_event(pointer_event);
    }

    /// The event loop supplies the calibrated raw-touch holding offset plus
    /// application rotation; panel-to-logical scaling happens exactly once.
    fn logical_position(&self, window: &MinimalSoftwareWindow) -> LogicalPosition {
        panel_to_logical_position(
            self.x,
            self.y,
            self.screen_width,
            self.screen_height,
            self.rotation,
            window.scale_factor(),
        )
    }
}

fn is_retryable(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
    )
}

fn checked_slot(value: i32) -> std::io::Result<i32> {
    if (0..MAX_TOUCH_SLOTS as i32).contains(&value) {
        Ok(value)
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid touch slot",
        ))
    }
}

fn checked_tracking_id(value: i32) -> std::io::Result<i32> {
    if value >= -1 {
        Ok(value)
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid touch tracking ID",
        ))
    }
}

impl Drop for TouchInput {
    fn drop(&mut self) {
        // Release the exclusive grab before closing
        unsafe { libc::ioctl(self.file_descriptor, EVIOCGRAB as _, 0 as libc::c_int) };
        unsafe { libc::close(self.file_descriptor) };
    }
}

pub(crate) fn has_accelerometer() -> bool {
    std::fs::read_to_string("/proc/bus/input/devices")
        .is_ok_and(|devices| lists_accelerometer(&devices))
}

fn lists_accelerometer(proc_devices: &str) -> bool {
    proc_devices
        .lines()
        .filter_map(|line| line.strip_prefix("B: ABS="))
        .any(|hex_words| {
            let axes = hex_words.split_whitespace().fold(0u64, |axes, word| {
                axes << 32 | u64::from_str_radix(word, 16).unwrap_or(0)
            });
            let reports = |axis: u16| axes & (1 << axis) != 0;
            reports(PRESSURE_AXIS)
                && !reports(SINGLE_TOUCH_POSITION_X)
                && !reports(TOUCH_POSITION_X)
        })
}

/// Slint rotates pixels on the way out but expects pointer events in the
/// un-rotated logical frame, and it exposes no inverse of its rotation, so
/// the mapping from panel pixel back to the app's frame has to live here.
fn undo_rotation(x: f32, y: f32, panel_width: f32, panel_height: f32, rotation: u32) -> (f32, f32) {
    match rotation {
        90 => (y, panel_width - x),
        180 => (panel_width - x, panel_height - y),
        270 => (panel_height - y, x),
        _ => (x, y),
    }
}

/// Touch axes are normalized to panel pixels. Undo rotation in that physical
/// frame before converting to the logical coordinates expected by Slint.
fn panel_to_logical_position(
    x: f32,
    y: f32,
    panel_width: f32,
    panel_height: f32,
    rotation: u32,
    scale_factor: f32,
) -> LogicalPosition {
    let (x, y) = undo_rotation(x, y, panel_width, panel_height, rotation);
    LogicalPosition::new(x / scale_factor, y / scale_factor)
}

#[cfg(test)]
mod tests {
    use super::{TouchInput, lists_accelerometer, panel_to_logical_position, undo_rotation};
    use slint::{ComponentHandle, LogicalPosition};

    slint::slint! {
        export component PointerCancelFixture inherits Window {
            width: 100px;
            height: 100px;
            callback activated();
            callback cancelled();
            callback released();
            out property <bool> pressed: contact.pressed;
            out property <float> pointer_x: contact.mouse-x / 1px;
            out property <float> pointer_y: contact.mouse-y / 1px;
            contact := TouchArea {
                width: parent.width;
                height: parent.height;
                clicked => { root.activated(); }
                pointer-event(event) => {
                    if event.kind == PointerEventKind.cancel { root.cancelled(); }
                    if event.kind == PointerEventKind.up { root.released(); }
                }
            }
        }
    }

    fn touch_pipe() -> (TouchInput, std::os::fd::OwnedFd) {
        use std::os::fd::FromRawFd;
        let mut descriptors = [-1; 2];
        // SAFETY: The initialized two-element array owns both returned descriptors.
        assert_eq!(
            unsafe { libc::pipe2(descriptors.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) },
            0
        );
        // SAFETY: pipe2 just created the writer, which has exactly one owner.
        let writer = unsafe { std::os::fd::OwnedFd::from_raw_fd(descriptors[1]) };
        let input = TouchInput {
            file_descriptor: descriptors[0],
            active_slot: 0,
            tracked_slot: None,
            x: 0.0,
            y: 0.0,
            pressed: false,
            screen_width: 100.0,
            screen_height: 100.0,
            rotation: 0,
            max_x: 100.0,
            max_y: 100.0,
            draining: true,
            drain_synchronization: super::DrainSynchronization::Complete,
            current_tracking: [-1; super::MAX_TOUCH_SLOTS],
            cancelled_tracking: [-1; super::MAX_TOUCH_SLOTS],
            legacy_held: false,
            cancelled_legacy: false,
            uses_tracking_ids: false,
            loss_coordinates: None,
        };
        (input, writer)
    }

    fn write_packet(writer: &std::os::fd::OwnedFd, kind: u16, code: u16, value: i32) {
        use std::os::fd::AsRawFd;
        let event = super::TouchInputEvent {
            timestamp_seconds: 0,
            timestamp_microseconds: 0,
            kind,
            code,
            value,
        };
        // SAFETY: The initialized native packet stays live for this pipe write.
        assert_eq!(
            unsafe {
                libc::write(
                    writer.as_raw_fd(),
                    (&event as *const super::TouchInputEvent).cast(),
                    std::mem::size_of_val(&event),
                )
            },
            std::mem::size_of_val(&event) as isize
        );
    }

    fn fresh_contacts(slot: usize, id: i32, x: i32, y: i32) -> super::ContactSnapshot {
        let mut tracking_ids = [-1; super::MAX_TOUCH_SLOTS];
        tracking_ids[slot] = id;
        let mut coordinates = [super::SlotCoordinates::EMPTY; super::MAX_TOUCH_SLOTS];
        coordinates[slot] = super::SlotCoordinates {
            x: Some(x),
            y: Some(y),
        };
        super::ContactSnapshot {
            active_slot: slot as i32,
            tracking_ids,
            legacy_held: false,
            uses_tracking_ids: true,
            coordinates,
        }
    }

    #[test]
    fn every_dropped_packet_boundary_requires_a_fresh_contact_snapshot() {
        let (mut input, writer) = touch_pipe();
        input.current_tracking[0] = 7;
        input.block_current_contacts();
        write_packet(&writer, super::EVENT_SYNC, super::SYNC_DROPPED, 0);
        write_packet(
            &writer,
            super::EVENT_ABSOLUTE_AXIS,
            super::TOUCH_TRACKING_ID,
            9,
        );
        write_packet(
            &writer,
            super::EVENT_ABSOLUTE_AXIS,
            super::TOUCH_POSITION_X,
            99,
        );
        assert_eq!(
            input
                .drain_cancelled_events_with(|_| { panic!("A snapshot must wait for SYN_REPORT") })
                .unwrap(),
            super::TransitionDrain::Waiting
        );
        assert!(input.draining);
        assert_eq!(input.cancelled_tracking[0], 7);
        assert_eq!(input.x, 0.0);

        write_packet(&writer, super::EVENT_SYNC, super::SYNC_REPORT, 0);
        let mut snapshots = 0;
        assert_eq!(
            input
                .drain_cancelled_events_with(|_| {
                    snapshots += 1;
                    if snapshots == 1 {
                        // A second loss queued during the first query cannot release
                        // the presentation gate using the earlier contact snapshot.
                        write_packet(&writer, super::EVENT_SYNC, super::SYNC_DROPPED, 0);
                        write_packet(
                            &writer,
                            super::EVENT_ABSOLUTE_AXIS,
                            super::TOUCH_TRACKING_ID,
                            11,
                        );
                        write_packet(&writer, super::EVENT_SYNC, super::SYNC_REPORT, 0);
                        Ok(fresh_contacts(1, 9, 70, 80))
                    } else {
                        Ok(fresh_contacts(2, 11, 40, 60))
                    }
                })
                .unwrap(),
            super::TransitionDrain::Ready
        );
        assert_eq!(snapshots, 2);
        assert_eq!(input.active_slot, 2);
        assert_eq!(input.current_tracking[2], 11);
        assert_eq!(input.cancelled_tracking[2], 11);
        assert!(input.cancelled_contact_is_held());
        assert_eq!(
            input.loss_coordinates.unwrap()[2].complete(),
            Some((40, 60))
        );
        assert_eq!(input.tracked_slot, None);
        assert!(!input.pressed);
    }

    #[test]
    fn a_retryable_contact_query_keeps_the_transition_gated() {
        let (mut input, writer) = touch_pipe();
        write_packet(&writer, super::EVENT_SYNC, super::SYNC_DROPPED, 0);
        write_packet(&writer, super::EVENT_SYNC, super::SYNC_REPORT, 0);
        for kind in [
            std::io::ErrorKind::Interrupted,
            std::io::ErrorKind::WouldBlock,
        ] {
            assert_eq!(
                input
                    .drain_cancelled_events_with(|_| { Err(std::io::Error::from(kind)) })
                    .unwrap(),
                super::TransitionDrain::Waiting
            );
            assert!(input.draining);
            assert_eq!(
                input.drain_synchronization,
                super::DrainSynchronization::SnapshotPending
            );
        }
        assert_eq!(
            input
                .drain_cancelled_events_with(|_| { Ok(fresh_contacts(0, 8, 75, 65)) })
                .unwrap(),
            super::TransitionDrain::Ready
        );
        assert!(!input.draining);
        assert_eq!(input.cancelled_tracking[0], 8);
        assert_eq!(input.tracked_slot, None);
    }

    #[test]
    fn only_eagain_completes_a_touch_drain_and_eof_or_short_packets_fail() {
        use std::os::fd::AsRawFd;
        let (mut input, writer) = touch_pipe();
        assert_eq!(
            input.finish_presentation_transition().unwrap(),
            super::TransitionDrain::Ready
        );
        drop(writer);
        assert_eq!(
            input.finish_presentation_transition().unwrap_err().kind(),
            std::io::ErrorKind::UnexpectedEof
        );
        assert!(input.draining);

        let (mut input, writer) = touch_pipe();
        let byte = 0u8;
        // SAFETY: This one-byte fixture deliberately creates an incomplete packet.
        assert_eq!(
            unsafe { libc::write(writer.as_raw_fd(), (&byte as *const u8).cast(), 1) },
            1
        );
        assert_eq!(
            input.finish_presentation_transition().unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
        assert!(input.draining);
    }

    #[test]
    fn an_active_flip_discards_old_packets_and_accepts_only_a_fresh_mapped_contact() {
        std::thread::spawn(|| {
            use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
            use slint::platform::{Platform, WindowAdapter};
            use std::{
                cell::Cell,
                os::fd::{AsRawFd, FromRawFd, OwnedFd},
                rc::Rc,
            };

            struct TestPlatform(Rc<MinimalSoftwareWindow>);
            impl Platform for TestPlatform {
                fn create_window_adapter(
                    &self,
                ) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
                    Ok(self.0.clone())
                }
            }
            let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
            slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).unwrap();
            let fixture = PointerCancelFixture::new().unwrap();
            window.set_size(slint::PhysicalSize::new(100, 100));
            fixture.show().unwrap();
            window.draw_if_needed(|renderer| {
                let mut pixels = vec![slint::Rgb8Pixel::default(); 100 * 100];
                renderer.render(&mut pixels, 100);
            });
            let clicks = Rc::new(Cell::new(0));
            let cancels = Rc::new(Cell::new(0));
            let recorded = clicks.clone();
            fixture.on_activated(move || recorded.set(recorded.get() + 1));
            let recorded = cancels.clone();
            fixture.on_cancelled(move || recorded.set(recorded.get() + 1));
            let mut descriptors = [-1; 2];
            // SAFETY: The two-element output array is initialized and live.
            assert_eq!(
                unsafe {
                    libc::pipe2(descriptors.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC)
                },
                0
            );
            // SAFETY: The writer was just allocated by pipe2 and has one owner.
            let writer = unsafe { OwnedFd::from_raw_fd(descriptors[1]) };
            let mut input = TouchInput {
                file_descriptor: descriptors[0],
                active_slot: 0,
                tracked_slot: None,
                x: 0.0,
                y: 0.0,
                pressed: false,
                screen_width: 100.0,
                screen_height: 100.0,
                rotation: 0,
                max_x: 100.0,
                max_y: 100.0,
                draining: false,
                drain_synchronization: super::DrainSynchronization::Complete,
                current_tracking: [-1; super::MAX_TOUCH_SLOTS],
                cancelled_tracking: [-1; super::MAX_TOUCH_SLOTS],
                legacy_held: false,
                cancelled_legacy: false,
                uses_tracking_ids: false,
                loss_coordinates: None,
            };
            let packet = |kind, code, value| {
                let event = super::TouchInputEvent {
                    timestamp_seconds: 0,
                    timestamp_microseconds: 0,
                    kind,
                    code,
                    value,
                };
                // SAFETY: The native event is fully initialized and live for
                // this bounded anonymous-pipe write; no device is involved.
                assert_eq!(
                    unsafe {
                        libc::write(
                            writer.as_raw_fd(),
                            (&event as *const super::TouchInputEvent).cast(),
                            std::mem::size_of_val(&event),
                        )
                    },
                    std::mem::size_of_val(&event) as isize
                );
            };
            packet(super::EVENT_ABSOLUTE_AXIS, super::TOUCH_TRACKING_ID, 7);
            packet(super::EVENT_SYNC, super::SYNC_REPORT, 0);
            input.poll(&window, 0).unwrap();
            assert!(
                !fixture.get_pressed(),
                "Unknown initial axes cannot create a press"
            );
            packet(super::EVENT_ABSOLUTE_AXIS, super::TOUCH_TRACKING_ID, 7);
            packet(super::EVENT_ABSOLUTE_AXIS, super::TOUCH_POSITION_X, 25);
            packet(super::EVENT_ABSOLUTE_AXIS, super::TOUCH_POSITION_Y, 25);
            packet(super::EVENT_SYNC, super::SYNC_REPORT, 0);
            input.poll(&window, 0).unwrap();
            assert!(fixture.get_pressed());
            packet(super::EVENT_ABSOLUTE_AXIS, super::TOUCH_TRACKING_ID, -1);
            packet(super::EVENT_ABSOLUTE_AXIS, super::TOUCH_TRACKING_ID, 8);
            packet(super::EVENT_ABSOLUTE_AXIS, super::TOUCH_POSITION_X, 90);
            packet(super::EVENT_ABSOLUTE_AXIS, super::TOUCH_POSITION_Y, 90);
            packet(super::EVENT_SYNC, super::SYNC_REPORT, 0);
            assert_eq!(
                input.cancel_and_drain(&window).unwrap(),
                super::TransitionDrain::Ready
            );
            assert_eq!(
                input.finish_presentation_transition().unwrap(),
                super::TransitionDrain::Ready
            );
            input.poll(&window, 180).unwrap();
            assert!(!fixture.get_pressed());
            assert_eq!((clicks.get(), cancels.get()), (0, 1));
            packet(super::EVENT_ABSOLUTE_AXIS, super::TOUCH_TRACKING_ID, 9);
            packet(super::EVENT_SYNC, super::SYNC_REPORT, 0);
            input.poll(&window, 180).unwrap();
            assert!(fixture.get_pressed());
            assert_eq!(
                (fixture.get_pointer_x(), fixture.get_pointer_y()),
                (10.0, 10.0)
            );
            packet(super::EVENT_ABSOLUTE_AXIS, super::TOUCH_TRACKING_ID, 9);
            packet(super::EVENT_ABSOLUTE_AXIS, super::TOUCH_POSITION_X, 75);
            packet(super::EVENT_ABSOLUTE_AXIS, super::TOUCH_POSITION_Y, 75);
            packet(super::EVENT_SYNC, super::SYNC_REPORT, 0);
            input.poll(&window, 180).unwrap();
            assert!(fixture.get_pressed());
            assert_eq!(
                (fixture.get_pointer_x(), fixture.get_pointer_y()),
                (25.0, 25.0)
            );
            packet(super::EVENT_ABSOLUTE_AXIS, super::TOUCH_TRACKING_ID, -1);
            packet(super::EVENT_SYNC, super::SYNC_REPORT, 0);
            input.poll(&window, 180).unwrap();
            assert_eq!(clicks.get(), 1);
            // A loss can change current slot axes without repeating unchanged
            // coordinates in the next genuine contact's event report.
            input.cancel(&window);
            input.draining = true;
            packet(super::EVENT_SYNC, super::SYNC_DROPPED, 0);
            packet(super::EVENT_SYNC, super::SYNC_REPORT, 0);
            assert_eq!(
                input
                    .drain_cancelled_events_with(|_| { Ok(fresh_contacts(0, 10, 50, 60)) })
                    .unwrap(),
                super::TransitionDrain::Ready
            );
            packet(super::EVENT_ABSOLUTE_AXIS, super::TOUCH_TRACKING_ID, 10);
            packet(super::EVENT_SYNC, super::SYNC_REPORT, 0);
            input.poll(&window, 180).unwrap();
            assert!(
                !fixture.get_pressed(),
                "The snapshotted held ID must stay cancelled"
            );
            packet(super::EVENT_ABSOLUTE_AXIS, super::TOUCH_TRACKING_ID, 11);
            packet(super::EVENT_SYNC, super::SYNC_REPORT, 0);
            input.poll(&window, 180).unwrap();
            assert!(fixture.get_pressed());
            assert_eq!(
                (fixture.get_pointer_x(), fixture.get_pointer_y()),
                (50.0, 40.0)
            );
            assert_eq!(clicks.get(), 1);
            packet(super::EVENT_ABSOLUTE_AXIS, super::TOUCH_TRACKING_ID, -1);
            packet(super::EVENT_SYNC, super::SYNC_REPORT, 0);
            input.poll(&window, 180).unwrap();
            assert_eq!(clicks.get(), 2);
            fixture.hide().unwrap();
        })
        .join()
        .unwrap();
    }

    #[test]
    fn managed_sleep_cancels_the_pointer_without_clicking_or_emitting_an_up() {
        std::thread::spawn(|| {
            use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
            use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};
            use std::{cell::Cell, os::fd::IntoRawFd, rc::Rc};

            struct TestPlatform(Rc<MinimalSoftwareWindow>);
            impl Platform for TestPlatform {
                fn create_window_adapter(
                    &self,
                ) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
                    Ok(self.0.clone())
                }
            }
            let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
            slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).unwrap();
            let fixture = PointerCancelFixture::new().unwrap();
            let clicks = Rc::new(Cell::new(0));
            let cancels = Rc::new(Cell::new(0));
            let releases = Rc::new(Cell::new(0));
            let recorded = clicks.clone();
            fixture.on_activated(move || recorded.set(recorded.get() + 1));
            let recorded = cancels.clone();
            fixture.on_cancelled(move || recorded.set(recorded.get() + 1));
            let recorded = releases.clone();
            fixture.on_released(move || recorded.set(recorded.get() + 1));
            window.set_size(slint::PhysicalSize::new(100, 100));
            fixture.show().unwrap();
            window.draw_if_needed(|renderer| {
                let mut pixels = vec![slint::Rgb8Pixel::default(); 100 * 100];
                renderer.render(&mut pixels, 100);
            });
            let mut input = TouchInput {
                file_descriptor: std::fs::File::open("/dev/null").unwrap().into_raw_fd(),
                active_slot: 0,
                tracked_slot: Some(0),
                x: 25.0,
                y: 25.0,
                pressed: false,
                screen_width: 100.0,
                screen_height: 100.0,
                rotation: 0,
                max_x: 100.0,
                max_y: 100.0,
                draining: false,
                drain_synchronization: super::DrainSynchronization::Complete,
                current_tracking: [-1; super::MAX_TOUCH_SLOTS],
                cancelled_tracking: [-1; super::MAX_TOUCH_SLOTS],
                legacy_held: false,
                cancelled_legacy: false,
                uses_tracking_ids: false,
                loss_coordinates: Some({
                    let mut coordinates = [super::SlotCoordinates::EMPTY; super::MAX_TOUCH_SLOTS];
                    // This direct-commit fixture supplies known physical axes.
                    coordinates[0] = super::SlotCoordinates {
                        x: Some(25),
                        y: Some(25),
                    };
                    coordinates
                }),
            };
            input.commit(&window);
            assert!(fixture.get_pressed());
            input.cancel(&window);
            assert!(!fixture.get_pressed());
            assert_eq!(clicks.get(), 0);
            assert_eq!(cancels.get(), 1);
            assert_eq!(releases.get(), 0);
            assert!(!input.pressed);
            assert_eq!(input.tracked_slot, None);
            input.cancel(&window);
            assert_eq!(
                cancels.get(),
                1,
                "Repeated cancellation cannot create a new gesture"
            );
            input.tracked_slot = Some(0);
            input.x = 25.0;
            input.y = 25.0;
            input.commit(&window);
            input.release(&window);
            assert_eq!(
                clicks.get(),
                1,
                "A fresh contact must work after the handoff"
            );
            assert_eq!(releases.get(), 1);
            assert!(!fixture.get_pressed());
            // A stale release after the cancellation cannot activate another item.
            window
                .try_dispatch_event(WindowEvent::PointerExited)
                .unwrap();
            window
                .try_dispatch_event(WindowEvent::PointerReleased {
                    position: LogicalPosition::new(25.0, 25.0),
                    button: PointerEventButton::Left,
                })
                .unwrap();
            assert_eq!(clicks.get(), 1);
            fixture.hide().unwrap();
        })
        .join()
        .expect("The isolated pointer-cancellation fixture must succeed");
    }

    #[test]
    fn undo_rotation_inverts_slints_rendering_rotation() {
        // Inputs are where Slint's clockwise RenderingRotation puts app pixel
        // (100, 300) on a portrait 1072x1448 panel.
        let (panel_width, panel_height) = (1072.0, 1448.0);
        assert_eq!(
            undo_rotation(100.0, 300.0, panel_width, panel_height, 0),
            (100.0, 300.0)
        );
        assert_eq!(
            undo_rotation(panel_width - 300.0, 100.0, panel_width, panel_height, 90),
            (100.0, 300.0)
        );
        assert_eq!(
            undo_rotation(
                panel_width - 100.0,
                panel_height - 300.0,
                panel_width,
                panel_height,
                180
            ),
            (100.0, 300.0)
        );
        assert_eq!(
            undo_rotation(300.0, panel_height - 100.0, panel_width, panel_height, 270),
            (100.0, 300.0)
        );
    }

    #[test]
    fn panel_coordinates_apply_window_scale_after_rotation() {
        // All four interior vectors represent logical (150, 270) at scale 2.
        // The rotated corners also catch scaling before undoing rotation.
        let (panel_width, panel_height) = (1072.0, 1448.0);
        let vectors = [
            (0, 300.0, 540.0, 150.0, 270.0),
            (90, 532.0, 300.0, 150.0, 270.0),
            (180, 772.0, 908.0, 150.0, 270.0),
            (270, 540.0, 1148.0, 150.0, 270.0),
            (0, 0.0, 0.0, 0.0, 0.0),
            (90, 1072.0, 0.0, 0.0, 0.0),
            (180, 1072.0, 1448.0, 0.0, 0.0),
            (270, 0.0, 1448.0, 0.0, 0.0),
        ];
        for (rotation, x, y, logical_x, logical_y) in vectors {
            assert_eq!(
                panel_to_logical_position(x, y, panel_width, panel_height, rotation, 2.0),
                LogicalPosition::new(logical_x, logical_y),
                "rotation {rotation}, panel position ({x}, {y})"
            );
        }
    }

    #[test]
    fn accelerometer_is_a_pressure_only_device() {
        assert!(lists_accelerometer("B: ABS=1000000\n"));
        // Touchscreen: pressure plus multitouch positions.
        assert!(!lists_accelerometer("B: ABS=260000 1000000\n"));
        // Pen: pressure plus x/y.
        assert!(!lists_accelerometer("B: ABS=1000003\n"));
    }
}
