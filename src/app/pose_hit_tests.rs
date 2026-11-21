use super::underline_projection_tests::{Fixture, isolated};
use super::*;
use anyhow::{Context, ensure};
use slint::{
    LogicalPosition, Model, PhysicalSize,
    platform::{PointerEventButton, WindowEvent},
};

const CASES: [&str; 5] = [
    "upright",
    "inverted",
    "changed_before_loop",
    "changed_after_attach",
    "wake_changed",
];
const TARGETS: [&str; 3] = ["mirror", "settings", "library"];
const SCRIBE_SIZE: PhysicalSize = PhysicalSize::new(1860, 2480);
const SCRIBE_SEARCH_BACK: LogicalPosition = LogicalPosition::new(56.0, 56.0);

fn tap(window: &slint::Window, position: LogicalPosition) -> Result<()> {
    window.try_dispatch_event(WindowEvent::PointerMoved { position })?;
    window.try_dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    })?;
    window.try_dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    })?;
    Ok(())
}

fn expect_route(fixture: &Fixture, route: Route) {
    let ui = fixture.ui();
    let shell = ui.global::<ShellModel>();
    assert_eq!(fixture.app().route, route);
    assert_eq!(shell.get_route().as_str(), route.name());
    if route == Route::Search {
        let search = ui.global::<SearchModel>();
        assert_eq!(search.get_purpose().as_str(), "books");
        assert_eq!(search.get_scope().as_str(), "store");
    } else {
        assert_eq!(shell.get_active_tab().as_str(), route.name());
        if route == Route::Library {
            assert!(ui.global::<LibraryModel>().get_books().row_count() > 0);
        }
    }
}

fn expect_painted_tabs(fixture: &Fixture, size: PhysicalSize, route: Route) -> Result<()> {
    let frame = fixture.ui().window().take_snapshot()?;
    assert_eq!((frame.width(), frame.height()), (size.width, size.height));
    let row = size.height - 70;
    for (column, selected) in [
        (size.width / 16, route == Route::Library),
        (13 * size.width / 16, route == Route::Settings),
    ] {
        let pixel = frame.as_slice()[(row * size.width + column) as usize];
        assert!(
            pixel.a == 255 && if selected { pixel.r < 32 } else { pixel.r > 224 },
            "{}: wrong painted tab at ({column}, {row}): {pixel:?}",
            route.name()
        );
    }
    Ok(())
}

fn verify_trip(
    fixture: &Fixture,
    [mirror, settings, library]: [LogicalPosition; 3],
    search_back: LogicalPosition,
    size: PhysicalSize,
) -> Result<()> {
    let window = fixture.ui().window();
    expect_route(fixture, Route::Library);
    tap(window, mirror)?;
    expect_route(fixture, Route::Search);
    tap(window, search_back)?;
    expect_route(fixture, Route::Library);
    tap(window, settings)?;
    expect_route(fixture, Route::Settings);
    expect_painted_tabs(fixture, size, Route::Settings)?;
    tap(window, library)?;
    expect_route(fixture, Route::Library);
    expect_painted_tabs(fixture, size, Route::Library)
}

fn replay_backend(
    fixture: &Fixture,
    size: PhysicalSize,
    search_back: LogicalPosition,
    trace: &str,
) -> Result<()> {
    let mut records = trace.lines();
    for case in CASES {
        let mut points = [LogicalPosition::default(); 3];
        for (index, target) in TARGETS.into_iter().enumerate() {
            let fields: Vec<_> = records
                .next()
                .unwrap_or_default()
                .split_ascii_whitespace()
                .collect();
            ensure!(
                fields.len() == 6 && fields[..4] == ["POSE_TOUCH", case, target, target],
                "Missing or unexpected backend contact for {case}/{target}"
            );
            let point = LogicalPosition::new(fields[4].parse()?, fields[5].parse()?);
            ensure!(
                (0.0..size.width as f32 / 2.0).contains(&point.x)
                    && (0.0..size.height as f32 / 2.0).contains(&point.y),
                "Invalid backend contact for {case}/{target}"
            );
            points[index] = point;
        }
        verify_trip(fixture, points, search_back, size)?;
        println!("POSE_UI {case} search library settings library");
    }
    ensure!(records.next().is_none(), "Extra backend contact record");
    Ok(())
}

fn with_window(
    profile: DeviceProfile,
    size: PhysicalSize,
    scenario: impl FnOnce(&Fixture) -> Result<()>,
) -> Result<()> {
    let fixture =
        Fixture::new_for_profile("Public offline navigation fixture.", vec![], false, profile)?;
    {
        let mut application = fixture.app();
        application.navigate(Route::Library, false)?;
        application.render()?;
    }
    let ui = fixture.ui();
    ui.show()?;
    let window = ui.window();
    window.try_dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 2.0 })?;
    window.set_size(size);
    assert_eq!(window.scale_factor(), 2.0);
    assert_eq!(window.size(), size);
    scenario(&fixture)?;
    fixture.finish()
}

#[test]
fn scribe_navigation_mirror_settings_and_library() -> Result<()> {
    isolated(
        "app::pose_hit_tests::scribe_navigation_mirror_settings_and_library",
        || {
            with_window(DeviceProfile::Scribe, SCRIBE_SIZE, |fixture| {
                verify_trip(
                    fixture,
                    [
                        LogicalPosition::new(117.0, 38.0),
                        LogicalPosition::new(813.0, 1202.0),
                        LogicalPosition::new(116.0, 1202.0),
                    ],
                    SCRIBE_SEARCH_BACK,
                    SCRIBE_SIZE,
                )
            })
        },
    )
}

#[test]
#[ignore = "requires WEREAD_POSE_TOUCH_TRACE from the Scribe backend acceptance test"]
fn scribe_replays_backend_touch_trace() -> Result<()> {
    isolated(
        "app::pose_hit_tests::scribe_replays_backend_touch_trace",
        || {
            let path = std::env::var_os("WEREAD_POSE_TOUCH_TRACE")
                .context("WEREAD_POSE_TOUCH_TRACE is required")?;
            let metadata = std::fs::symlink_metadata(&path)?;
            ensure!(
                metadata.is_file() && (1..=4096).contains(&metadata.len()),
                "Expected a nonempty bounded backend trace file"
            );
            let trace = std::fs::read_to_string(path)?;
            with_window(DeviceProfile::Scribe, SCRIBE_SIZE, |fixture| {
                replay_backend(fixture, SCRIBE_SIZE, SCRIBE_SEARCH_BACK, &trace)
            })
        },
    )
}

#[test]
fn paperwhite6_navigation_mirror_settings_and_library() -> Result<()> {
    isolated(
        "app::pose_hit_tests::paperwhite6_navigation_mirror_settings_and_library",
        || verify_paperwhite6(PhysicalSize::new(1264, 1680)),
    )
}

#[test]
fn experimental_paperwhite6_geometry_navigation_mirror_settings_and_library() -> Result<()> {
    isolated(
        "app::pose_hit_tests::experimental_paperwhite6_geometry_navigation_mirror_settings_and_library",
        || verify_paperwhite6(PhysicalSize::new(1272, 1696)),
    )
}

fn verify_paperwhite6(size: PhysicalSize) -> Result<()> {
    let width = size.width as f32 / 2.0;
    let height = size.height as f32 / 2.0;
    with_window(DeviceProfile::Paperwhite6, size, |fixture| {
        verify_trip(
            fixture,
            [
                LogicalPosition::new(width / 8.0, 40.0),
                LogicalPosition::new(width * 7.0 / 8.0, height - 40.0),
                LogicalPosition::new(width / 8.0, height - 40.0),
            ],
            LogicalPosition::new(width - 32.0, 56.0),
            size,
        )
    })
}
