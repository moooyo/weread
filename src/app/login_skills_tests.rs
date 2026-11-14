//! Skills guidance fences authentication work without replacing private state.

use super::super::library::with_qr_input_test_limit;
use super::super::underline_projection_tests::{BOOK, Fixture, isolated};
use super::*;
use crate::domain::{Annotation, ReadingPosition};
use crate::jobs::{JobEvent, JobResult};
use crate::source::{AuthSession, QrLoginChallenge, QrLoginState, SourceError};
use slint::Model;

const PUBLIC_SETUP_URL: &str = "https://weread.qq.com/r/weread-skills";

fn challenge(label: &str) -> QrLoginChallenge {
    QrLoginChallenge {
        uid: format!("synthetic-skills-{label}"),
        confirm_url: format!("https://example.invalid/confirm?uid=synthetic-skills-{label}"),
        expires_at: chrono::Utc::now().timestamp() + 300,
    }
}

fn image_bytes(image: &slint::Image) -> Vec<u8> {
    image
        .to_rgba8()
        .map(|pixels| pixels.as_bytes().to_vec())
        .unwrap_or_default()
}

fn assert_public_setup_qr(image: &slint::Image) -> Result<()> {
    assert_eq!(SKILLS_SETUP_URL, PUBLIC_SETUP_URL);
    let uri = url::Url::parse(PUBLIC_SETUP_URL)?;
    assert!(uri.username().is_empty() && uri.password().is_none());
    assert!(uri.query().is_none() && uri.fragment().is_none());
    let expected = qrcode::QrCode::new(PUBLIC_SETUP_URL.as_bytes())?;
    let edge = expected.width() + 8;
    let scale = SKILLS_QR_MODULE_SCALE as usize;
    let raster_edge = edge * scale;
    let pixels = image
        .to_rgba8()
        .expect("The setup QR must retain its actual encoded pixel buffer");
    assert_eq!(
        (pixels.width(), pixels.height()),
        (raster_edge as u32, raster_edge as u32)
    );
    for (index, pixel) in pixels.as_slice().iter().enumerate() {
        let (x, y) = (index % raster_edge / scale, index / raster_edge / scale);
        let dark = x >= 4
            && y >= 4
            && x < edge - 4
            && y < edge - 4
            && expected[(x - 4, y - 4)] == qrcode::Color::Dark;
        assert_eq!(
            (pixel.r, pixel.g, pixel.b, pixel.a),
            (0, 0, 0, if dark { 255 } else { 0 }),
            "The guide must encode only the public setup URI at {x},{y}"
        );
    }
    Ok(())
}

fn prepare(fixture: &Fixture) -> Result<()> {
    let mut state = fixture.app();
    state.demo = false;
    state.force_offline = true;
    state.online = false;
    state.reader.session_started = None;
    state.prefs.report_time = false;
    state.prefs.push_on_close = false;
    state.prefs.keep_awake = false;
    let session = AuthSession {
        api_key: "synthetic-skills-retained-key".into(),
        user_vid: "synthetic-skills-retained-account".into(),
        account_name: "Skills guidance fixture account".into(),
        ..AuthSession::default()
    };
    state.session_file.save(&session)?;
    state.jobs.replace_session(session.clone())?;
    state.persisted_session = Some(session);
    state.store.save_annotation(&Annotation {
        id: "skills-retained-note".into(),
        book_id: BOOK.into(),
        chapter_index: 0,
        start: 0,
        end: 6,
        quote: "Reader".into(),
        note: "Retain the existing synthetic note".into(),
        created_at: 1,
    })?;
    state.route = Route::Login;
    state.begin_login();
    state.render()?;
    assert!(!fixture.ui().global::<LoginModel>().get_skills_open());
    Ok(())
}

#[derive(Debug, PartialEq)]
struct SavedData {
    session: Option<AuthSession>,
    persisted_session: Option<AuthSession>,
    account_file: Vec<u8>,
    account_epoch: u64,
    preferences: serde_json::Value,
    chapter: Option<String>,
    annotations: Vec<Annotation>,
    position: Option<ReadingPosition>,
    reader_body: String,
}

impl SavedData {
    fn read(state: &App) -> Result<Self> {
        Ok(Self {
            session: state.jobs.session(),
            persisted_session: state.persisted_session.clone(),
            account_file: std::fs::read(state.data_dir.join("account.json"))?,
            account_epoch: state.jobs.account_epoch(),
            preferences: serde_json::to_value(&state.prefs)?,
            chapter: state.store.chapter_body(BOOK, 0)?,
            annotations: state.store.annotations(BOOK)?,
            position: state.store.position(BOOK)?,
            reader_body: state.reader.body.clone(),
        })
    }
}

#[derive(Debug, PartialEq)]
struct LoginScope {
    route: Route,
    request: u64,
    next_request: u64,
    polling: bool,
    challenge: Option<(String, String, i64)>,
    code: String,
    key: String,
    step: String,
    busy: bool,
    status: String,
    qr_label: String,
    qr: Vec<u8>,
    skills_open: bool,
    skills_qr: Vec<u8>,
    visible_code: String,
    visible_key: String,
    digits: Vec<String>,
    can_submit: bool,
}

impl LoginScope {
    fn read(state: &App, ui: &AppWindow) -> Self {
        let login = ui.global::<LoginModel>();
        Self {
            route: state.route.clone(),
            request: state.login_request,
            next_request: state.next_id,
            polling: state.login_polling,
            challenge: state.login_challenge.as_ref().map(|value| {
                (
                    value.uid.clone(),
                    value.confirm_url.clone(),
                    value.expires_at,
                )
            }),
            code: state.login_code.clone(),
            key: state.login_key.clone(),
            step: login.get_step().to_string(),
            busy: login.get_busy(),
            status: login.get_status().to_string(),
            qr_label: login.get_qr_label().to_string(),
            qr: image_bytes(&login.get_qr()),
            skills_open: login.get_skills_open(),
            skills_qr: image_bytes(&login.get_skills_qr()),
            visible_code: login.get_code().to_string(),
            visible_key: login.get_api_key().to_string(),
            digits: login
                .get_digits()
                .iter()
                .map(|digit| digit.to_string())
                .collect(),
            can_submit: login.get_can_submit(),
        }
    }
}

fn seed_pending_login(state: &mut App, ui: &AppWindow) -> Result<u64> {
    state.login_request = state.request();
    state.login_challenge = Some(challenge("pending"));
    state.login_polling = true;
    state.login_code = "1234".into();
    state.login_key = "synthetic-unsubmitted-skills-key".into();
    let login = ui.global::<LoginModel>();
    login.set_step("code".into());
    login.set_busy(true);
    login.set_status("Pending fixture login".into());
    login.set_qr(super::super::library::qr_image(
        "https://example.invalid/retained-login-qr",
    )?);
    state.render_login();
    Ok(state.login_request)
}

fn help_and_back() -> Result<()> {
    let fixture = Fixture::new_for_profile(
        &"Reader Skills guidance fixture. ".repeat(80),
        Vec::new(),
        false,
        DeviceProfile::Scribe,
    )?;
    prepare(&fixture)?;
    {
        let mut state = fixture.app();
        let saved = SavedData::read(&state)?;
        let before_back = LoginScope::read(&state, fixture.ui());
        state.login_action("skills-back", "")?;
        assert_eq!(LoginScope::read(&state, fixture.ui()), before_back);
        let failure_request = seed_pending_login(&mut state, fixture.ui())?;
        let failure_epoch = state.jobs.account_epoch();
        assert!(state.handle_job(JobEvent::new(
            failure_request,
            failure_epoch,
            JobResult::LoginPoll(Err(SourceError::Unsupported("skill_not_enabled"))),
        ))?);
        let failed = fixture.ui().global::<LoginModel>();
        assert_eq!(failed.get_step().as_str(), "qr");
        assert_eq!(failed.get_status().as_str(), tr("login_skill_required"));
        assert!(!failed.get_skills_open() && !failed.get_busy());
        assert!(!failed.get_can_submit());
        assert!(failed.get_code().is_empty() && failed.get_api_key().is_empty());
        assert!(failed.get_digits().iter().all(|digit| digit.is_empty()));
        assert_eq!(failed.get_qr().size().width, 0);
        assert!(state.login_challenge.is_none() && !state.login_polling);
        assert!(state.login_code.is_empty() && state.login_key.is_empty());
        assert_eq!(SavedData::read(&state)?, saved);
        let old_request = seed_pending_login(&mut state, fixture.ui())?;
        let epoch = state.jobs.account_epoch();
        state.dispatch("login", "skills-help", "")?;
        let login = fixture.ui().global::<LoginModel>();
        assert_eq!(state.route, Route::Login);
        assert!(!state.closing.get());
        assert!(login.get_skills_open());
        assert_public_setup_qr(&login.get_skills_qr())?;
        let guide_pixels = image_bytes(&login.get_skills_qr());
        assert_ne!(state.login_request, old_request);
        assert!(!state.login_polling && state.login_challenge.is_none());
        assert!(state.login_code.is_empty() && state.login_key.is_empty());
        assert!(login.get_code().is_empty() && login.get_api_key().is_empty());
        assert!(login.get_digits().iter().all(|digit| digit.is_empty()));
        assert_eq!(login.get_code_length(), 0);
        assert!(!login.get_busy() && !login.get_can_submit());
        assert!(login.get_qr().size().width == 0);
        assert!(login.get_qr_label().is_empty() && login.get_status().is_empty());

        for result in [
            JobResult::LoginBegin(Ok(challenge("cancelled-begin"))),
            JobResult::LoginPoll(Ok(QrLoginState::NeedCode)),
        ] {
            assert!(!state.handle_job(JobEvent::new(old_request, epoch, result))?);
            state.render_login();
            assert!(login.get_skills_open());
            assert_eq!(image_bytes(&login.get_skills_qr()), guide_pixels);
            assert!(login.get_qr().size().width == 0);
            assert_eq!(login.get_step().as_str(), "qr");
            assert!(login.get_status().is_empty());
        }
        state.poll_login();
        assert!(!state.login_polling && state.login_challenge.is_none());
        assert_eq!(SavedData::read(&state)?, saved);

        let guide_request = state.login_request;
        state.dispatch("login", "skills-back", "")?;
        assert_eq!(state.route, Route::Login);
        assert!(!state.closing.get());
        assert!(!login.get_skills_open());
        assert_ne!(state.login_request, guide_request);
        assert_ne!(state.login_request, old_request);
        assert!(!state.login_polling && state.login_challenge.is_none());
        assert_eq!(login.get_step().as_str(), "qr");
        assert!(login.get_qr().size().width == 0);
        assert!(!login.get_busy());
        assert_eq!(login.get_status().as_str(), tr("network_unavailable"));
        assert!(state.force_offline && !state.online);

        let fresh_request = state.login_request;
        assert!(!state.handle_job(JobEvent::new(
            old_request,
            epoch,
            JobResult::LoginBegin(Ok(challenge("late-after-back"))),
        ))?);
        assert!(state.handle_job(JobEvent::new(
            fresh_request,
            epoch,
            JobResult::LoginBegin(Ok(challenge("fresh-after-back"))),
        ))?);
        assert!(!login.get_skills_open());
        assert_eq!(login.get_step().as_str(), "qr");
        assert!(login.get_qr().size().width > 0);
        assert_eq!(
            state.login_challenge.as_ref().unwrap().uid,
            "synthetic-skills-fresh-after-back"
        );
        assert_eq!(SavedData::read(&state)?, saved);
    }
    fixture.finish()
}

fn rejected_help_is_atomic() -> Result<()> {
    let fixture = Fixture::new("Reader Skills rejection fixture.", Vec::new(), false)?;
    prepare(&fixture)?;
    {
        let mut state = fixture.app();
        seed_pending_login(&mut state, fixture.ui())?;
        let saved = SavedData::read(&state)?;
        let before = LoginScope::read(&state, fixture.ui());
        with_qr_input_test_limit(0, || {
            assert!(state.login_action("skills-help", "").is_err());
        });
        assert_eq!(LoginScope::read(&state, fixture.ui()), before);
        assert_eq!(SavedData::read(&state)?, saved);
    }
    fixture.finish()
}

fn ignored_outside_login() -> Result<()> {
    let fixture = Fixture::new("Reader Skills route fixture.", Vec::new(), false)?;
    prepare(&fixture)?;
    {
        let mut state = fixture.app();
        seed_pending_login(&mut state, fixture.ui())?;
        let saved = SavedData::read(&state)?;
        for route in [Route::Library, Route::Reader(BOOK.into(), 0)] {
            state.route = route;
            for guide_open in [false, true] {
                fixture
                    .ui()
                    .global::<LoginModel>()
                    .set_skills_open(guide_open);
                let before = LoginScope::read(&state, fixture.ui());
                // A zero encoder budget makes accidental route-bypassing work fail.
                with_qr_input_test_limit(0, || -> Result<()> {
                    state.login_action("skills-help", "")?;
                    state.login_action("skills-back", "")?;
                    Ok(())
                })?;
                assert_eq!(LoginScope::read(&state, fixture.ui()), before);
                assert!(!state.closing.get());
                assert_eq!(SavedData::read(&state)?, saved);
            }
        }
    }
    fixture.finish()
}

#[test]
fn login_skills_help_fences_old_results_and_back_restarts_qr_without_replacing_account()
-> Result<()> {
    isolated(
        "app::auth::login_skills_tests::login_skills_help_fences_old_results_and_back_restarts_qr_without_replacing_account",
        help_and_back,
    )
}

#[test]
fn login_skills_encoding_refusal_preserves_the_pending_login_scope() -> Result<()> {
    isolated(
        "app::auth::login_skills_tests::login_skills_encoding_refusal_preserves_the_pending_login_scope",
        rejected_help_is_atomic,
    )
}

#[test]
fn login_skills_actions_outside_login_do_not_encode_or_mutate_state() -> Result<()> {
    isolated(
        "app::auth::login_skills_tests::login_skills_actions_outside_login_do_not_encode_or_mutate_state",
        ignored_outside_login,
    )
}
