#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    fn region() -> Option<crate::PhysicalRect> {
        Some(crate::PhysicalRect { x: 7, y: 11, width: 801, height: 901 })
    }

    fn completed(_: &File, marker: u32) -> bool { marker == 13 }

    fn send(file: &File, marker: u32, wait: fn(&File, u32) -> bool) {
        accepted(file, marker, false, 257, [7, 11, 801, 901], wait);
    }

    fn initial_frame() -> File {
        OBSERVER.with_borrow_mut(|slot| *slot = None);
        assert!(enable());
        let file = File::open("/dev/null").unwrap();
        new_iteration();
        draw_begin(false);
        draw_end();
        written(region(), false);
        send(&file, 9, completed);
        assert_eq!(natural_frames(), 1);
        file
    }

    fn receipt() -> Frame {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(frame) = poll(1).unwrap() { return frame; }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
    }

    #[test]
    fn qualifying_draw_keeps_the_issued_marker_and_actual_waveform() {
        let file = initial_frame();
        new_iteration();
        assert!(!arm(2, 3, 144, || true));
        assert!(arm(1, 3, 144, || true));
        assert!(!arm(1, 3, 144, || true));
        draw_begin(true);
        draw_end();
        written(region(), true);
        send(&file, 13, completed);
        send(&file, 14, completed);
        let frame = receipt();
        assert_eq!((frame.marker, frame.wait_marker, frame.target_page, frame.target_anchor), (13, Some(13), 3, 144));
        assert_eq!((frame.full, frame.waveform, frame.rect), (false, 257, [7, 11, 801, 901]));
        assert!(frame.draw_begin_ns <= frame.draw_end_ns && frame.draw_end_ns <= frame.written_ns);
        assert!(frame.written_ns <= frame.sent_ns && frame.sent_ns <= frame.wait_begin_ns.unwrap());
        assert!(frame.wait_begin_ns <= frame.wait_end_ns);
        assert_eq!(frame.wait_status, "acknowledged");
    }

    #[test]
    fn later_frames_changed_models_and_empty_writes_cannot_satisfy_a_turn() {
        let file = initial_frame();
        new_iteration();
        assert!(arm(1, 3, 144, || true));
        new_iteration();
        draw_begin(true);
        draw_end();
        written(region(), true);
        send(&file, 13, completed);
        assert!(poll(1).is_err());

        for changed in [true, false] {
            let file = initial_frame();
            new_iteration();
            let target = Rc::new(Cell::new(true));
            let page = target.clone();
            assert!(arm(1, 3, 144, move || page.get()));
            draw_begin(true);
            draw_end();
            target.set(!changed);
            written(if changed { region() } else { None }, true);
            send(&file, 13, completed);
            assert!(poll(1).is_err());
        }
    }

    #[test]
    fn rejected_wait_preserves_send_and_never_creates_a_completion() {
        let file = initial_frame();
        new_iteration();
        assert!(arm(1, 3, 144, || true));
        draw_begin(true);
        draw_end();
        written(region(), true);
        send(&file, 13, |_, _| false);
        let frame = receipt();
        assert_eq!(frame.marker, 13);
        assert_eq!(frame.wait_marker, None);
        assert_eq!(frame.wait_begin_ns, None);
        assert_eq!(frame.wait_end_ns, None);
        assert_eq!(frame.wait_status, "not-witnessed");
    }

    #[test]
    fn periodic_full_is_an_ordinary_draw_with_an_independent_marker_wait() {
        let file = initial_frame();
        new_iteration();
        assert!(arm(1, 7, 906, || true));
        draw_begin(true);
        draw_end();
        written(region(), true);
        accepted(&file, 13, true, 2, [0, 0, 1860, 2480], completed);
        let frame = receipt();
        assert!(frame.full);
        assert_eq!((frame.waveform, frame.rect, frame.wait_marker), (2, [0, 0, 1860, 2480], Some(13)));
        assert_eq!(natural_frames(), 2);
    }

    #[test]
    fn forced_draws_and_rejected_updates_do_not_create_target_receipts() {
        for redraw in [true, false] {
            let file = initial_frame();
            new_iteration();
            assert!(arm(1, 3, 144, || true));
            draw_begin(!redraw);
            draw_end();
            written(region(), !redraw);
            if redraw { send(&file, 13, completed); } else { new_iteration(); }
            assert!(poll(1).is_err());
        }
    }

    #[test]
    fn the_wait_thread_starts_once_only_after_the_first_valid_arm() {
        OBSERVER.with_borrow_mut(|slot| *slot = None);
        assert!(enable());
        assert!(!arm(1, 2, 144, || true));
        let status = || OBSERVER.with_borrow(|slot| {
            let observer = slot.as_ref().unwrap();
            (observer.wait_attempted, observer.waiter.is_some())
        });
        assert_eq!(status(), (false, false));
        let file = initial_frame();
        new_iteration();
        assert!(!arm(2, 2, 144, || true));
        assert!(!arm(1, 2, 144, || false));
        assert_eq!(status(), (false, false));
        assert!(arm(1, 2, 144, || true));
        assert_eq!(status(), (true, true));
        draw_begin(true);
        draw_end();
        written(region(), true);
        send(&file, 13, |_, _| false);
        assert_eq!(receipt().wait_status, "not-witnessed");
        assert_eq!(status(), (true, false));
        new_iteration();
        assert!(arm(2, 3, 322, || true));
        assert_eq!(status(), (true, false));
    }
}
