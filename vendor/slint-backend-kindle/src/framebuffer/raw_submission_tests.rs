//! Synthetic Y4 ABI and uncertain-delivery regressions; no device is accessed.

use super::tests::{observed_night_mode, update_test_framebuffer};
use super::*;
use std::cell::RefCell;

fn framebuffer() -> Framebuffer {
    let framebuffer = update_test_framebuffer(observed_night_mode(1, false, false));
    framebuffer.update_variant.set(Some(UpdateVariant::Mtk));
    framebuffer
}

fn visible(framebuffer: &Framebuffer) -> [u8; 4] {
    // SAFETY: the isolated fixture owns a live anonymous mapping of 4096 bytes.
    unsafe { std::slice::from_raw_parts(framebuffer.map, 4) }
        .try_into()
        .unwrap()
}

#[test]
fn y4_packet_uses_only_the_reviewed_mtk_flag_and_parameters() {
    let framebuffer = framebuffer();
    let PackedUpdateRequest::Mtk(request) = framebuffer
        .pack_raw_update_request(
            UpdateVariant::Mtk,
            UpdateRect::default(),
            UPDATE_MODE_FULL,
            7,
            true,
        )
        .unwrap()
    else {
        panic!("Hardware Y4 must use the MTK ABI");
    };
    assert_eq!(request.waveform_mode, WAVEFORM_MODE_GC16);
    assert_eq!(request.update_marker, 7);
    assert_eq!(request.flags, 0x4000);
    assert_eq!(
        request.flags & 0x8000,
        0,
        "REGAL is a different MTK operation"
    );
    assert_eq!((request.dither_mode, request.quant_bit), (0, 0));
    assert_eq!(request.hist_bw_waveform_mode, WAVEFORM_MODE_DU);
    assert_eq!(request.hist_gray_waveform_mode, WAVEFORM_MODE_GC16);
    for variant in [
        UpdateVariant::Legacy,
        UpdateVariant::Rex,
        UpdateVariant::Zelda,
    ] {
        assert!(
            framebuffer
                .pack_raw_update_request(
                    variant,
                    UpdateRect::default(),
                    UPDATE_MODE_PARTIAL,
                    8,
                    true
                )
                .is_none()
        );
    }
}

#[test]
fn eligibility_requires_accepted_mtk_gray8_and_actual_noninverted_output() {
    let mut framebuffer = framebuffer();
    assert!(framebuffer.hardware_y4_supported());
    for variant in [
        None,
        Some(UpdateVariant::Legacy),
        Some(UpdateVariant::Rex),
        Some(UpdateVariant::Zelda),
    ] {
        framebuffer.update_variant.set(variant);
        assert!(!framebuffer.hardware_y4_supported());
    }
    framebuffer.update_variant.set(Some(UpdateVariant::Mtk));
    framebuffer.y4_metadata_supported = false;
    assert!(!framebuffer.hardware_y4_supported());
    framebuffer.y4_metadata_supported = true;
    framebuffer.bytes_per_pixel = 4;
    assert!(!framebuffer.hardware_y4_supported());
    framebuffer.bytes_per_pixel = 1;
    // Native inversion can be active even when application hardware writes are disabled.
    framebuffer.night_mode = observed_night_mode(2, false, false);
    assert!(!framebuffer.hardware_y4_supported());
    framebuffer.night_mode = observed_night_mode(1, true, true);
    assert!(!framebuffer.hardware_y4_supported());
    framebuffer.night_mode = observed_night_mode(2, true, false);
    assert!(framebuffer.hardware_y4_supported());
}

#[test]
fn software_raw_night_updates_keep_the_existing_gck16_policy_without_y4() {
    let framebuffer = update_test_framebuffer(observed_night_mode(1, true, true));
    let PackedUpdateRequest::Mtk(request) = framebuffer
        .pack_raw_update_request(
            UpdateVariant::Mtk,
            UpdateRect::default(),
            UPDATE_MODE_FULL,
            9,
            false,
        )
        .unwrap()
    else {
        panic!("The fixture requests the MTK ABI");
    };
    assert_eq!(request.waveform_mode, WAVEFORM_MODE_GCK16_MTK);
    assert_eq!(request.flags, 0);
    assert_eq!((request.dither_mode, request.quant_bit), (0, 0));
}

#[test]
fn explicit_hardware_rejection_keeps_the_old_success_until_a_fresh_software_marker() {
    for errno in [libc::EINVAL, libc::ENOTTY, libc::EOPNOTSUPP] {
        let framebuffer = framebuffer();
        assert!(framebuffer.submit_update_with(|_, marker| {
            assert_eq!(marker, 1);
            true
        }));
        let rejected = framebuffer
            .submit_raw_update_with(true, |variant, marker| {
                assert_eq!((variant, marker), (UpdateVariant::Mtk, 2));
                Err(std::io::Error::from_raw_os_error(errno))
            })
            .unwrap();
        assert!(!rejected.accepted);
        assert_eq!((rejected.attempts, rejected.marker), (1, Some(2)));
        assert!(rejected.hardware_y4_attempted && !rejected.hardware_y4_submitted);
        assert_eq!(rejected.last_errno, Some(errno));
        assert_eq!(framebuffer.last_submitted_marker.get(), Some(1));
        assert!(framebuffer.require_known_dithering_delivery().is_ok());
        let events = RefCell::new(Vec::new());
        framebuffer
            .raw_completion_with(|| {
                framebuffer.wait_for_update_complete_with(|record| {
                    assert_eq!(record.update_marker, 1);
                    events.borrow_mut().push("complete-old-marker");
                    Ok(())
                })
            })
            .unwrap();
        events.borrow_mut().push("rewrite-current-raw-regions");
        framebuffer.reset_raw_abi_after_rejection(Some(errno));
        let software = framebuffer
            .submit_raw_update_with(false, |_, marker| {
                assert_eq!(marker, 3);
                events.borrow_mut().push("submit-new-software-marker");
                Ok(())
            })
            .unwrap();
        assert!(software.accepted && !software.hardware_y4_submitted);
        assert_eq!(software.marker, Some(3));
        assert_eq!(
            &*events.borrow(),
            &[
                "complete-old-marker",
                "rewrite-current-raw-regions",
                "submit-new-software-marker"
            ]
        );
    }
}

#[test]
fn software_raw_probes_only_continue_after_definite_rejections() {
    let framebuffer = framebuffer();
    framebuffer.reset_raw_abi_after_rejection(Some(libc::EINVAL));
    assert_eq!(framebuffer.update_variant.get(), Some(UpdateVariant::Mtk));
    framebuffer.reset_raw_abi_after_rejection(Some(libc::ENOTTY));
    assert_eq!(framebuffer.update_variant.get(), None);
    let mut attempts = Vec::new();
    let accepted = framebuffer
        .submit_raw_update_with(false, |variant, marker| {
            attempts.push((variant, marker));
            match variant {
                UpdateVariant::Legacy => Err(std::io::Error::from_raw_os_error(libc::EINVAL)),
                UpdateVariant::Rex => Err(std::io::Error::from_raw_os_error(libc::ENOTTY)),
                UpdateVariant::Zelda => Err(std::io::Error::from_raw_os_error(libc::EOPNOTSUPP)),
                UpdateVariant::Mtk => Ok(()),
                UpdateVariant::Unsupported => panic!("Unsupported is not an ioctl ABI"),
            }
        })
        .unwrap();
    assert_eq!(
        attempts,
        UpdateVariant::PROBE_ORDER.map(|variant| (variant, 1))
    );
    assert!(accepted.accepted);
    assert_eq!(accepted.attempts, 4);
    assert_eq!(framebuffer.last_submitted_marker.get(), Some(1));
    assert_eq!(framebuffer.update_variant.get(), Some(UpdateVariant::Mtk));
}

#[test]
fn uncertain_raw_delivery_stops_probing_and_permanently_blocks_mapping_operations() {
    for errno in [
        Some(libc::EIO),
        Some(libc::EINTR),
        Some(libc::ETIMEDOUT),
        None,
    ] {
        for hardware in [false, true] {
            let mut framebuffer = framebuffer();
            framebuffer.fill(213);
            if !hardware {
                framebuffer.update_variant.set(None);
            }
            let mut attempts = 0;
            assert!(
                framebuffer
                    .submit_raw_update_with(hardware, |_, _| {
                        attempts += 1;
                        Err(errno
                            .map(std::io::Error::from_raw_os_error)
                            .unwrap_or_else(|| std::io::Error::other("unknown ioctl outcome")))
                    })
                    .is_err()
            );
            assert_eq!(attempts, 1);
            assert_eq!(framebuffer.last_issued_marker.get(), 1);
            assert_eq!(framebuffer.last_submitted_marker.get(), None);
            if !hardware {
                assert_eq!(framebuffer.update_variant.get(), None);
            }
            assert_eq!(framebuffer.dithering_delivery_failure.get(), Some(errno));
            framebuffer.fill(2);
            framebuffer.write_line(0, 0..2, &[1, 2]);
            assert_eq!(visible(&framebuffer), [213; 4]);
            assert!(!framebuffer.submit_update_with(|_, _| panic!("Normal writes cannot retry")));
            assert!(
                framebuffer
                    .submit_raw_update_with(false, |_, _| panic!("Raw writes cannot retry"))
                    .is_err()
            );
            assert!(framebuffer.refresh_full_checked().is_err());
            assert!(framebuffer.wait_for_update_complete_checked().is_err());
            assert!(
                framebuffer
                    .wait_for_pending_update_complete_checked()
                    .is_err()
            );
            assert!(framebuffer.wait_for_raw_update_complete_checked().is_err());
            framebuffer.reset_raw_abi_after_rejection(Some(libc::ENOTTY));
            assert!(framebuffer.require_known_dithering_delivery().is_err());
        }
    }
}

#[test]
fn failed_raw_completion_latches_even_an_explicit_wait_abi_error() {
    for errno in [libc::EIO, libc::EINTR, libc::ETIMEDOUT, libc::ENOTTY] {
        let mut framebuffer = framebuffer();
        framebuffer.fill(199);
        assert!(
            framebuffer
                .submit_raw_update_with(true, |_, _| Ok(()))
                .unwrap()
                .accepted
        );
        assert!(
            framebuffer
                .raw_completion_with(|| Err(std::io::Error::from_raw_os_error(errno)))
                .is_err()
        );
        assert_eq!(framebuffer.last_completed_marker.get(), None);
        framebuffer.write_line(0, 0..2, &[0, 0]);
        assert_eq!(visible(&framebuffer), [199; 4]);
        assert!(
            framebuffer
                .raw_completion_with(|| panic!("A later completion cannot clear the latch"))
                .is_err()
        );
    }
}

#[test]
fn raw_regions_align_outward_in_both_axes_and_clip_to_physical_edges() {
    for (input, expected) in [
        ((9, 10, 2, 3), (8, 8, 8, 8)),
        ((-3, -4, 5, 6), (0, 0, 8, 8)),
        ((16, 18, u32::MAX, u32::MAX), (16, 16, 1, 3)),
        ((17, 0, 5, 5), (0, 0, 0, 0)),
    ] {
        let region = raw_refresh_region(
            PhysicalRect {
                x: input.0,
                y: input.1,
                width: input.2,
                height: input.3,
            },
            17,
            19,
        );
        assert_eq!(
            (region.left, region.top, region.width, region.height),
            expected
        );
    }
}

#[test]
fn empty_raw_updates_do_not_consume_markers_and_exhaustion_cannot_wrap() {
    let framebuffer = framebuffer();
    let empty = framebuffer
        .refresh_raw_checked(
            ResolvedRefresh {
                region: PhysicalRect {
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 2,
                },
                waveform: RefreshWaveform::A2,
                flashing: false,
            },
            true,
        )
        .unwrap();
    assert_eq!(empty, RefreshSubmission::default());
    assert_eq!(framebuffer.last_issued_marker.get(), 0);
    framebuffer.wait_for_raw_update_complete_checked().unwrap();
    framebuffer.last_issued_marker.set(u32::MAX);
    let error = framebuffer
        .submit_raw_update_with(true, |_, _| panic!("A marker cannot wrap"))
        .unwrap_err();
    assert_eq!(error.raw_os_error(), Some(libc::EOVERFLOW));
    assert_eq!(framebuffer.last_submitted_marker.get(), None);
}

#[test]
fn metadata_gate_requires_valid_complete_gray8_layout() {
    let valid = ModeSnapshot {
        variable: FbVarScreeninfo {
            xres: 17,
            yres: 19,
            xres_virtual: 17,
            yres_virtual: 19,
            bits_per_pixel: 8,
            grayscale: 1,
            ..Default::default()
        },
        fixed: FbFixScreeninfo {
            line_length: 24,
            smem_len: 24 * 19,
            ..Default::default()
        },
    };
    assert!(hardware_y4_metadata_supported(valid));
    let mut inverted = valid;
    inverted.variable.grayscale = 2;
    assert!(hardware_y4_metadata_supported(inverted));
    for mutate in [
        (|snapshot: &mut ModeSnapshot| snapshot.variable.bits_per_pixel = 32)
            as fn(&mut ModeSnapshot),
        |snapshot| snapshot.variable.grayscale = 0,
        |snapshot| snapshot.variable.xoffset = 1,
        |snapshot| snapshot.variable.yoffset = 1,
        |snapshot| snapshot.variable.xres_virtual = 16,
        |snapshot| snapshot.variable.rotate = 4,
        |snapshot| snapshot.fixed.line_length = 16,
        |snapshot| snapshot.fixed.smem_len = 24 * 19 - 1,
    ] {
        let mut invalid = valid;
        mutate(&mut invalid);
        assert!(!hardware_y4_metadata_supported(invalid));
    }
}

fn planned_packet_summary(request: &PackedUpdateRequest) -> ((u32, u32, u32, u32), u32, u32, u32) {
    let (region, waveform, mode, flags) = match request {
        PackedUpdateRequest::Legacy(update) => (
            update.update_region,
            update.waveform_mode,
            update.update_mode,
            update.flags,
        ),
        PackedUpdateRequest::Rex(update) => (
            update.update_region,
            update.waveform_mode,
            update.update_mode,
            update.flags,
        ),
        PackedUpdateRequest::Zelda(update) => (
            update.update_region,
            update.waveform_mode,
            update.update_mode,
            update.flags,
        ),
        PackedUpdateRequest::Mtk(update) => (
            update.update_region,
            update.waveform_mode,
            update.update_mode,
            update.flags,
        ),
    };
    (
        (region.left, region.top, region.width, region.height),
        waveform,
        mode,
        flags,
    )
}

#[test]
fn checked_ordinary_updates_preserve_the_unaligned_region_and_original_waveform() {
    for variant in UpdateVariant::PROBE_ORDER {
        for waveform in [
            RefreshWaveform::Auto,
            RefreshWaveform::Du,
            RefreshWaveform::A2,
            RefreshWaveform::Gc16,
        ] {
            let framebuffer = framebuffer();
            framebuffer.update_variant.set(Some(variant));
            let expected = waveform_for_driver(waveform, Some(variant));
            framebuffer
                .refresh_planned_checked_with(
                    ResolvedRefresh {
                        region: PhysicalRect {
                            x: 3,
                            y: 5,
                            width: 2,
                            height: 3,
                        },
                        waveform,
                        flashing: false,
                    },
                    |request| {
                        let summary = planned_packet_summary(request);
                        assert_eq!(
                            summary.0,
                            (3, 5, 2, 3),
                            "Ordinary regions must not receive Y4 alignment"
                        );
                        assert_eq!((summary.1, summary.2), (expected, UPDATE_MODE_PARTIAL));
                        assert_eq!(summary.3 & MTK_EPDC_FLAG_USE_DITHERING_Y4, 0);
                        assert_eq!(
                            summary.3 & EPDC_FLAG_FORCE_MONOCHROME,
                            if waveform == RefreshWaveform::Du {
                                EPDC_FLAG_FORCE_MONOCHROME
                            } else {
                                0
                            }
                        );
                        Ok(())
                    },
                )
                .unwrap();
            assert_eq!(framebuffer.last_submitted_marker.get(), Some(1));
        }
    }
}

#[test]
fn checked_ordinary_first_a2_probe_keeps_auto_and_flashing_night_keeps_gck16() {
    let framebuffer = framebuffer();
    framebuffer.update_variant.set(None);
    let mut attempts = 0;
    framebuffer
        .refresh_planned_checked_with(
            ResolvedRefresh {
                region: PhysicalRect {
                    x: 3,
                    y: 5,
                    width: 2,
                    height: 3,
                },
                waveform: RefreshWaveform::A2,
                flashing: false,
            },
            |request| {
                attempts += 1;
                assert_eq!(planned_packet_summary(request).1, WAVEFORM_MODE_AUTO);
                if attempts < 4 {
                    Err(std::io::Error::from_raw_os_error(libc::ENOTTY))
                } else {
                    Ok(())
                }
            },
        )
        .unwrap();
    assert_eq!(attempts, 4);
    let night = update_test_framebuffer(observed_night_mode(1, true, true));
    night.update_variant.set(Some(UpdateVariant::Mtk));
    night
        .refresh_planned_checked_with(
            ResolvedRefresh {
                region: PhysicalRect {
                    x: 1,
                    y: 1,
                    width: 1,
                    height: 1,
                },
                waveform: RefreshWaveform::A2,
                flashing: true,
            },
            |request| {
                let summary = planned_packet_summary(request);
                assert_eq!(
                    summary,
                    ((1, 1, 1, 1), WAVEFORM_MODE_GCK16_MTK, UPDATE_MODE_FULL, 0)
                );
                Ok(())
            },
        )
        .unwrap();
}

#[test]
fn checked_ordinary_errors_propagate_exact_errno_and_uncertain_delivery_blocks_retry() {
    for errno in [libc::EIO, libc::EINTR, libc::ETIMEDOUT] {
        let framebuffer = framebuffer();
        framebuffer.update_variant.set(None);
        let plan = ResolvedRefresh {
            region: PhysicalRect {
                x: 1,
                y: 1,
                width: 1,
                height: 1,
            },
            waveform: RefreshWaveform::Du,
            flashing: false,
        };
        let mut attempts = 0;
        let error = framebuffer
            .refresh_planned_checked_with(plan, |_| {
                attempts += 1;
                Err(std::io::Error::from_raw_os_error(errno))
            })
            .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(errno));
        assert_eq!(attempts, 1);
        assert_eq!(framebuffer.update_variant.get(), None);
        assert_eq!(
            framebuffer.dithering_delivery_failure.get(),
            Some(Some(errno))
        );
        assert!(
            framebuffer
                .refresh_planned_checked_with(plan, |_| panic!("A later plan cannot retry"))
                .is_err()
        );
    }
    let framebuffer = framebuffer();
    let rejected = framebuffer
        .refresh_planned_checked_with(
            ResolvedRefresh {
                region: PhysicalRect {
                    x: 1,
                    y: 1,
                    width: 1,
                    height: 1,
                },
                waveform: RefreshWaveform::Gc16,
                flashing: false,
            },
            |_| Err(std::io::Error::from_raw_os_error(libc::EINVAL)),
        )
        .unwrap_err();
    assert_eq!(rejected.raw_os_error(), Some(libc::EINVAL));
    assert!(framebuffer.require_known_dithering_delivery().is_ok());
}
