//! Opt-in monotonic startup markers for matched device measurements.

use std::{
    io::Write,
    sync::{
        OnceLock,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};

use slint::platform::software_renderer::RenderingRotation;

pub const STARTUP_DRAW_TRACE_POLICY: &str = "first-application-render-begin-v1";

static ENABLED: OnceLock<bool> = OnceLock::new();
static FIRST_FRAME: AtomicU8 = AtomicU8::new(0);

fn enabled() -> bool {
    *ENABLED.get_or_init(|| std::env::var("WEREAD_STARTUP_TIMING").ok().as_deref() == Some("1"))
}

/// Emit only fixed stage names and a device-local monotonic timestamp.
/// Disabled ordinary launches perform no clock query or diagnostic write.
pub fn mark(stage: &'static str) {
    if enabled() {
        write_mark(format_args!("{stage}"));
    }
}

pub(crate) fn mark_orientation(site: &'static str, phase: &'static str) {
    if enabled() {
        write_mark(format_args!("{site}_orientation_{phase}"));
    }
}

fn write_mark(stage: std::fmt::Arguments<'_>) {
    if let Some(time) = device_time() {
        let _ = write_at(&mut std::io::stderr().lock(), time, stage);
    }
}

fn device_time() -> Option<Duration> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: time is the complete writable timespec used by CLOCK_MONOTONIC.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } != 0 {
        return None;
    }
    Some(Duration::new(
        time.tv_sec.try_into().ok()?,
        time.tv_nsec.try_into().ok()?,
    ))
}

fn write_at(
    output: &mut impl Write,
    time: Duration,
    stage: impl std::fmt::Display,
) -> std::io::Result<()> {
    writeln!(
        output,
        "WEREAD_STARTUP {}.{:09} {stage}",
        time.as_secs(),
        time.subsec_nanos()
    )
}

pub(crate) struct FirstDraw {
    time: Duration,
    stage: &'static str,
}

impl FirstDraw {
    pub(crate) fn begin(
        ordinary_initial: bool,
        app_rotation: u32,
        rotation: RenderingRotation,
    ) -> Option<Self> {
        Self::begin_with(
            ordinary_initial,
            app_rotation,
            rotation,
            enabled,
            device_time,
        )
    }

    fn begin_with(
        ordinary_initial: bool,
        app_rotation: u32,
        rotation: RenderingRotation,
        enabled: impl FnOnce() -> bool,
        clock: impl FnOnce() -> Option<Duration>,
    ) -> Option<Self> {
        if !ordinary_initial || app_rotation != 0 || !enabled() {
            return None;
        }
        let stage = match rotation {
            RenderingRotation::NoRotation => "first_application_frame_draw_begin_rot0",
            RenderingRotation::Rotate90 => "first_application_frame_draw_begin_rot90",
            RenderingRotation::Rotate180 => "first_application_frame_draw_begin_rot180",
            RenderingRotation::Rotate270 => "first_application_frame_draw_begin_rot270",
            _ => return None,
        };
        Some(Self {
            time: clock()?,
            stage,
        })
    }
}

pub(crate) fn arm_first_application_frame() {
    if enabled() {
        let _ = FIRST_FRAME.compare_exchange(0, 1, Ordering::Relaxed, Ordering::Relaxed);
    }
}

pub(crate) fn update_submitted(draw: Option<&FirstDraw>) {
    record_submission(&FIRST_FRAME, draw, device_time, |draw, submitted| {
        let _ = write_submission(&mut std::io::stderr().lock(), draw, submitted);
    });
}

fn write_submission(
    output: &mut impl Write,
    draw: Option<&FirstDraw>,
    submitted: Duration,
) -> std::io::Result<()> {
    if let Some(draw) = draw {
        write_at(output, draw.time, draw.stage)?;
    }
    write_at(output, submitted, "first_application_frame_send")
}

fn record_submission(
    state: &AtomicU8,
    draw: Option<&FirstDraw>,
    clock: impl FnOnce() -> Option<Duration>,
    emit: impl FnOnce(Option<&FirstDraw>, Duration),
) {
    if state
        .compare_exchange(1, 2, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
        && let Some(submitted) = clock()
    {
        emit(draw.filter(|draw| draw.time < submitted), submitted);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use RenderingRotation::{NoRotation, Rotate90, Rotate180, Rotate270};

    fn stamp(nanos: u32) -> Duration {
        Duration::new(41, nanos)
    }

    fn submit(state: &AtomicU8, draw: Option<&FirstDraw>, nanos: u32, output: &mut Vec<u8>) {
        record_submission(
            state,
            draw,
            || Some(stamp(nanos)),
            |draw, sent| write_submission(output, draw, sent).unwrap(),
        );
    }

    #[test]
    fn qualified_renderer_angles_emit_one_adjacent_pair_without_resampling_the_draw() {
        for (rotation, angle) in [
            (NoRotation, 0),
            (Rotate90, 90),
            (Rotate180, 180),
            (Rotate270, 270),
        ] {
            let state = AtomicU8::new(0);
            let draw =
                FirstDraw::begin_with(true, 0, rotation, || true, || Some(stamp(3))).unwrap();
            let mut output = Vec::new();
            record_submission(&state, None, || panic!("splash SEND"), |_, _| panic!());
            state.store(1, Ordering::Relaxed);
            submit(&state, Some(&draw), 19, &mut output);
            record_submission(
                &state,
                Some(&draw),
                || panic!("later SEND"),
                |_, _| panic!(),
            );
            assert_eq!(
                String::from_utf8(output).unwrap(),
                format!(
                    "WEREAD_STARTUP 41.000000003 first_application_frame_draw_begin_rot{angle}\n\
                     WEREAD_STARTUP 41.000000019 first_application_frame_send\n"
                )
            );
        }
    }

    #[test]
    fn disabled_nonordinary_and_nonzero_base_draws_never_read_the_clock() {
        for (enabled, ordinary, base) in [
            (false, true, 0),
            (true, false, 0),
            (true, true, 90),
            (true, true, 180),
            (true, true, 270),
        ] {
            let draw = FirstDraw::begin_with(ordinary, base, Rotate180, || enabled, || panic!());
            assert!(draw.is_none());
        }
        assert!(FirstDraw::begin_with(true, 0, Rotate180, || true, || None).is_none());
    }

    #[test]
    fn bypass_or_lost_draw_clock_keeps_send_but_cannot_gain_a_witness_later() {
        let draw = FirstDraw::begin_with(true, 0, Rotate180, || true, || Some(stamp(20))).unwrap();
        for (first, nanos) in [(None, 30), (Some(&draw), 20), (Some(&draw), 19)] {
            let state = AtomicU8::new(1);
            let mut output = Vec::new();
            submit(&state, first, nanos, &mut output);
            submit(&state, Some(&draw), 99, &mut output);
            assert_eq!(
                String::from_utf8(output).unwrap(),
                format!("WEREAD_STARTUP 41.{nanos:09} first_application_frame_send\n")
            );
        }
        let state = AtomicU8::new(1);
        record_submission(&state, Some(&draw), || None, |_, _| panic!());
        record_submission(&state, Some(&draw), || panic!(), |_, _| panic!());
    }
}
