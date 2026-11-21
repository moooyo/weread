use super::underline_projection_tests::{Fixture, isolated};
use super::*;
use crate::platform::LibraryFrameContent::{Empty, Entries};
use crate::platform::LibraryFrameState::{Cancelled, Ready, Waiting};
use slint::Model;

#[test]
fn library_frame_uses_the_projected_shelf_and_never_promotes_a_placeholder() -> Result<()> {
    isolated(
        "app::library_frame_tests::library_frame_uses_the_projected_shelf_and_never_promotes_a_placeholder",
        || {
            let fixture = Fixture::new("Synthetic text.", vec![], false)?;
            let mut state = fixture.app();
            state.route = Route::Library;
            state.render()?;
            state.arm_startup_library();
            assert_eq!(state.startup_library_frame(), Cancelled);
            state.demo = false;
            state.arm_startup_library();
            state.render()?;
            let shell = fixture.ui().global::<ShellModel>();
            let library = fixture.ui().global::<LibraryModel>();
            shell.set_toast("".into());
            assert_eq!(state.startup_library_frame(), Ready(Entries));
            let original = library.get_books();
            let mut entry = original.row_data(0).unwrap();
            entry.has_cover = false;
            library.set_books(model(vec![entry.clone()]));
            for mode in ["list", "cover"] {
                library.set_view_mode(mode.into());
                assert_eq!(state.startup_library_frame(), Ready(Entries));
            }
            entry.title = "  ".into();
            library.set_books(model(vec![entry]));
            assert_eq!(state.startup_library_frame(), Waiting);
            library.set_books(original);
            for other in ["login", "reader", "detail", "settings"] {
                shell.set_route(other.into());
                assert_eq!(state.startup_library_frame(), Cancelled);
            }
            shell.set_route("library".into());
            shell.set_active_tab("settings".into());
            assert_eq!(state.startup_library_frame(), Cancelled);
            shell.set_active_tab("library".into());
            shell.set_dialog_kind("confirm".into());
            assert_eq!(state.startup_library_frame(), Waiting);
            shell.set_dialog_kind("".into());
            shell.set_toast("Synthetic notice".into());
            assert_eq!(state.startup_library_frame(), Waiting);
            shell.set_toast("".into());
            fixture.ui().global::<SleepScreenModel>().set_active(true);
            assert_eq!(state.startup_library_frame(), Cancelled);
            fixture.ui().global::<SleepScreenModel>().set_active(false);
            library.set_selecting(true);
            assert_eq!(state.startup_library_frame(), Cancelled);
            library.set_selecting(false);
            state.busy = true;
            assert_eq!(state.startup_library_frame(), Ready(Entries));
            let empty = tempfile::tempdir()?;
            let original_store = std::mem::replace(
                &mut state.store,
                Store::open(empty.path().join("empty.sqlite"))?,
            );
            state.render_library()?;
            library.set_busy(false);
            assert_eq!(state.startup_library_frame(), Waiting);
            state.busy = false;
            assert_eq!(state.startup_library_frame(), Ready(Empty));
            state.library_sort = "downloaded".into();
            state.render_library()?;
            assert_eq!(state.startup_library_frame(), Waiting);
            state.library_sort = "default".into();
            state.library_status = "Synthetic failure".into();
            state.render_library()?;
            assert_eq!(state.startup_library_frame(), Waiting);
            state.library_status.clear();
            state.render_library()?;
            library.set_summary("".into());
            assert_eq!(state.startup_library_frame(), Waiting);
            state.store = original_store;
            state.render_library()?;
            assert_eq!(state.startup_library_frame(), Ready(Entries));
            state.dispatch("library", "view", "")?;
            assert_eq!(state.startup_library_frame(), Cancelled);
            state.arm_startup_library();
            state.navigate(Route::Settings, true)?;
            state.navigate(Route::Library, false)?;
            assert_eq!(state.startup_library_frame(), Cancelled);
            drop(state);
            fixture.finish()
        },
    )
}
