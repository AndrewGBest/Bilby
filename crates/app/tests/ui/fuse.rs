//! AppImages on systems without FUSE 2.

use super::*;

/// The notice about FUSE is about AppImages. An app that is here as something else
/// (its Flatpak) says nothing about it, even though it also comes as an AppImage.
#[test]
#[cfg(unix)]
fn the_fuse_notice_is_only_for_an_appimage() {
    use bilby_core::needs::Fuse;
    use std::os::unix::fs::PermissionsExt;
    init_backend();
    bilby_core::demo::DOWNLOAD_STEP_MS.store(0, Ordering::Relaxed);
    let dirs = dirs_in(&temp_root());
    std::fs::create_dir_all(&dirs.config).unwrap();
    std::fs::write(dirs.config.join("demo-state.json"), r#"{"settings":{"welcomed":true}}"#).unwrap();
    // A stand-in for Flatpak that installs the bundle as the real one does.
    let apps = dirs.flatpak_roots[0].join("app");
    std::fs::create_dir_all(dirs.flatpak.parent().unwrap()).unwrap();
    std::fs::write(
        &dirs.flatpak,
        format!(
            "#!/bin/sh\nif [ \"$1\" = install ]; then echo 'Installing app/org.driftnotes.Drift/x86_64/stable'; mkdir -p '{}/org.driftnotes.Drift'; fi\n",
            apps.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&dirs.flatpak, std::fs::Permissions::from_mode(0o755)).unwrap();
    let options = Options {
        demo: true,
        inline: true,
        platform: Platform::preset("linux-x64-arch"),
        shell: Some(Shell::Desktop),
        dirs: Some(dirs.clone()),
        opener: Some(Arc::new(|_: &std::path::Path| Ok(()))),
        fuse: Some(Fuse { library: false, mounter: true }),
        ..Default::default()
    };
    let (ui, app) = build(&options).unwrap();
    let store = ui.global::<Store>();

    // Not installed, and the AppImage is what Install would fetch: the notice is there.
    app.open(&ui, "github:driftnotes/drift");
    let labels: Vec<String> = store.get_options().iter().map(|o| o.label.to_string()).collect();
    assert!(labels[0].contains("AppImage"), "{labels:?}");
    assert!(store.get_need_note().contains("FUSE 2"), "{}", store.get_need_note());

    // Installed as its Flatpak instead: nothing about FUSE, here or on coming back.
    let bundle = labels.iter().position(|l| l.contains("Flatpak")).expect("the bundle is offered") as i32;
    app.install(&ui, bundle);
    assert_eq!(store.get_install_state(), 2, "{}", store.get_install_message());
    assert!(store.get_can_open());
    assert_eq!(store.get_need_note(), "");
    app.open(&ui, "github:driftnotes/drift");
    assert_eq!(store.get_need_note(), "");
}

#[test]
fn an_appimage_on_a_system_without_fuse_has_it_installed_along_with_the_app() {
    use bilby_core::needs::Fuse;
    let start_with_fuse = |fuse: Fuse| {
        init_backend();
        bilby_core::demo::DOWNLOAD_STEP_MS.store(0, Ordering::Relaxed);
        let dirs = dirs_in(&temp_root());
        std::fs::create_dir_all(&dirs.config).unwrap();
        std::fs::write(dirs.config.join("demo-state.json"), r#"{"settings":{"welcomed":true}}"#).unwrap();
        // A stand-in for the password prompt: it notes what it was asked to run and succeeds.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::create_dir_all(dirs.elevate.parent().unwrap()).unwrap();
            let log = dirs.elevate.with_extension("log");
            std::fs::write(&dirs.elevate, format!("#!/bin/sh\necho \"$@\" >> '{}'\n", log.display())).unwrap();
            std::fs::set_permissions(&dirs.elevate, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let options = Options {
            demo: true,
            inline: true,
            // An Arch system: its repositories have the package that provides FUSE 2.
            platform: Platform::preset("linux-x64-arch"),
            shell: Some(Shell::Desktop),
            dirs: Some(dirs.clone()),
            opener: Some(Arc::new(|_: &std::path::Path| Ok(()))),
            fuse: Some(fuse),
            ..Default::default()
        };
        let (ui, app) = build(&options).unwrap();
        (ui, app, dirs)
    };
    // A demo app whose best file on this system is an AppImage.
    let without = Fuse { library: false, mounter: true };
    let (ui, app, dirs) = start_with_fuse(without);
    let store = ui.global::<Store>();
    let id = ["github:tidewave/tidewave", "github:lumen-labs/lumen", "github:kestrel-sh/kestrel"]
        .into_iter()
        .find(|id| {
            app.open(&ui, id);
            store.get_options().row_data(0).is_some_and(|o| o.label.contains("AppImage"))
        })
        .expect("the demo has an app that comes as an AppImage on Arch");

    // Before anything is downloaded the page says what is needed and that the store
    // will see to it. There is no errand to run: no button leading elsewhere.
    let note = store.get_need_note();
    assert!(note.contains("needs FUSE 2") && note.contains("Bilby will install it for you"), "{note}");
    assert_eq!(store.get_helper_id(), "");

    // Installing the app installs FUSE first, through the system's package manager.
    #[cfg(unix)]
    {
        store.invoke_install(0);
        let ran = std::fs::read_to_string(dirs.elevate.with_extension("log")).unwrap_or_default();
        assert!(ran.contains("pacman") && ran.contains("fuse2"), "the package was installed first: {ran:?}");
        assert_eq!(store.get_install_state(), 2, "and then the app: {}", store.get_install_message());
        assert_eq!(store.get_need_note(), "", "with it in place, nothing more is said");
    }
    let _ = (&dirs, id);

    // The page never sends the person off to install something themselves.
    assert!(!note.contains("Install your system"), "{note}");

    // With FUSE in place to begin with, nothing is said and nothing extra is run.
    let (ui, app, dirs) = start_with_fuse(Fuse { library: true, mounter: true });
    app.open(&ui, id);
    assert_eq!(ui.global::<Store>().get_need_note(), "");
    ui.global::<Store>().invoke_install(0);
    assert!(!dirs.elevate.with_extension("log").exists(), "no package manager was called");
}

#[test]
fn where_fuse_cannot_be_installed_the_app_is_unpacked_instead_of_asking_the_person() {
    use bilby_core::needs::Fuse;
    init_backend();
    bilby_core::demo::DOWNLOAD_STEP_MS.store(0, Ordering::Relaxed);
    let dirs = dirs_in(&temp_root());
    std::fs::create_dir_all(&dirs.config).unwrap();
    std::fs::write(dirs.config.join("demo-state.json"), r#"{"settings":{"welcomed":true}}"#).unwrap();
    // A system whose repositories have no FUSE 2 package at all.
    let options = Options {
        demo: true,
        inline: true,
        platform: Platform::preset("linux-x64-fedora"),
        shell: Some(Shell::Desktop),
        dirs: Some(dirs.clone()),
        opener: Some(Arc::new(|_: &std::path::Path| Ok(()))),
        fuse: Some(Fuse { library: false, mounter: true }),
        ..Default::default()
    };
    let (ui, app) = build(&options).unwrap();
    let store = ui.global::<Store>();
    let Some(id) =
        ["github:tidewave/tidewave", "github:lumen-labs/lumen", "github:kestrel-sh/kestrel"].into_iter().find(|id| {
            app.open(&ui, id);
            store.get_options().row_data(0).is_some_and(|o| o.label.contains("AppImage"))
        })
    else {
        return; // no AppImage is the best file on this made-up system: nothing to check
    };
    let note = store.get_need_note();
    assert!(note.contains("Bilby will unpack it") && note.contains("Nothing for you to do"), "{note}");
    assert!(
        !note.contains("Install your system") && store.get_helper_id().is_empty(),
        "no errand for the person: {note}"
    );
    // Installing goes ahead without a package manager or a password.
    store.invoke_install(0);
    assert_eq!(store.get_install_state(), 2, "{}", store.get_install_message());
    assert!(!dirs.elevate.with_extension("log").exists(), "nothing was asked of the system");
    let _ = id;
}

// A read-only system (Fedora Kinoite and its kin) has FUSE added on top of its image.
#[cfg(target_os = "linux")]
#[test]
fn on_a_read_only_system_fuse_is_added_and_a_restart_is_offered() {
    use bilby_core::needs::Fuse;
    use std::os::unix::fs::PermissionsExt;
    init_backend();
    bilby_core::demo::DOWNLOAD_STEP_MS.store(0, Ordering::Relaxed);
    let dirs = dirs_in(&temp_root());
    std::fs::create_dir_all(&dirs.config).unwrap();
    std::fs::write(dirs.config.join("demo-state.json"), r#"{"settings":{"welcomed":true}}"#).unwrap();
    // A stand-in for rpm-ostree that notes what it was asked and succeeds.
    let tool = dirs.config.join("rpm-ostree");
    let asked = dirs.config.join("asked");
    std::fs::write(&tool, format!("#!/bin/sh\necho \"$@\" >> '{}'\n", asked.display())).unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    let restarts: Arc<Mutex<u32>> = Arc::default();
    let count = restarts.clone();
    let started: Arc<Mutex<Vec<PathBuf>>> = Arc::default();
    let record = started.clone();
    let options = Options {
        demo: true,
        inline: true,
        platform: Platform::preset("linux-x64-fedora"),
        shell: Some(Shell::Desktop),
        dirs: Some(dirs.clone()),
        opener: Some(Arc::new(|_: &std::path::Path| Ok(()))),
        launcher: Some(Arc::new(move |path: &std::path::Path| {
            record.lock().unwrap().push(path.to_path_buf());
            Ok(())
        })),
        fuse: Some(Fuse { library: false, mounter: true }),
        layering: Some(tool),
        restart: Some(Arc::new(move || {
            *count.lock().unwrap() += 1;
            Ok(())
        })),
        ..Default::default()
    };
    let (ui, app) = build(&options).unwrap();
    let store = ui.global::<Store>();
    let Some(id) =
        ["github:tidewave/tidewave", "github:lumen-labs/lumen", "github:kestrel-sh/kestrel"].into_iter().find(|id| {
            app.open(&ui, id);
            store.get_options().row_data(0).is_some_and(|o| o.label.contains("AppImage"))
        })
    else {
        return;
    };
    let note = store.get_need_note();
    assert!(note.contains("Bilby will add it to your system") && note.contains("restart"), "{note}");
    store.invoke_install(0);
    assert_eq!(store.get_install_state(), 2, "{}", store.get_install_message());
    // The made-up download is not a real AppImage. Make it one that asks for FUSE 2.
    let placed = std::fs::read_dir(&dirs.applications)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "AppImage"))
        .expect("the app was placed");
    std::fs::write(&placed, b"\x7fELF....libfuse.so.2....").unwrap();
    let _ = std::fs::remove_file(&asked);
    app.open(&ui, id);
    assert!(store.get_need_note().contains("Bilby will add it to your system"), "{}", store.get_need_note());

    // Opening it adds FUSE through the system's own tool, and asks about restarting.
    store.invoke_launch();
    assert_eq!(std::fs::read_to_string(&asked).unwrap().trim(), "install -y --idempotent fuse fuse-libs");
    assert_eq!(store.get_gate(), 1, "a question is on screen");
    assert_eq!((store.get_gate_no().as_str(), store.get_gate_yes().as_str()), ("Restart later", "Restart now"));
    assert!(store.get_gate_text().contains("after a restart"));
    assert!(started.lock().unwrap().is_empty(), "it is not started only to fail");
    assert!(placed.exists(), "and it is left as the file it is, not unpacked");

    // Later: nothing restarts, and the page says where things stand.
    store.invoke_gate_answer(false);
    assert_eq!((store.get_gate(), *restarts.lock().unwrap()), (0, 0));
    assert!(store.get_need_note().contains("has been added"), "{}", store.get_need_note());

    // Opening it again before restarting asks again, without adding anything twice.
    store.invoke_launch();
    assert_eq!(store.get_gate(), 1);
    assert_eq!(std::fs::read_to_string(&asked).unwrap().lines().count(), 1);
    store.invoke_gate_answer(true);
    assert_eq!((store.get_gate(), *restarts.lock().unwrap()), (0, 1), "Restart now restarts");

    // The age question still has its own words afterwards.
    assert!(started.lock().unwrap().is_empty());
}

// The copy that is running proves its kind of file starts here.
#[cfg(target_os = "linux")]
#[test]
fn bilby_updating_itself_is_never_unpacked_or_made_to_wait_on_fuse() {
    use bilby_core::needs::Fuse;
    let start_as = |store_app: &str| {
        init_backend();
        bilby_core::demo::DOWNLOAD_STEP_MS.store(0, Ordering::Relaxed);
        let dirs = dirs_in(&temp_root());
        std::fs::create_dir_all(&dirs.config).unwrap();
        std::fs::write(dirs.config.join("demo-state.json"), r#"{"settings":{"welcomed":true}}"#).unwrap();
        let options = Options {
            demo: true,
            inline: true,
            // No FUSE 2 and no package for it: any other AppImage would be unpacked.
            platform: Platform::preset("linux-x64-fedora"),
            shell: Some(Shell::Desktop),
            dirs: Some(dirs.clone()),
            opener: Some(Arc::new(|_: &std::path::Path| Ok(()))),
            launcher: Some(Arc::new(|_: &std::path::Path| Ok(()))),
            fuse: Some(Fuse { library: false, mounter: true }),
            store_app: Some(store_app.to_string()),
            ..Default::default()
        };
        let (ui, app) = build(&options).unwrap();
        (ui, app, dirs)
    };
    let Some(stand_in) =
        ["github:tidewave/tidewave", "github:lumen-labs/lumen", "github:kestrel-sh/kestrel"].into_iter().find(|id| {
            let (ui, app, _) = start_as(id);
            app.open(&ui, id);
            ui.global::<Store>().get_options().row_data(0).is_some_and(|o| o.label.contains("AppImage"))
        })
    else {
        return;
    };
    let (ui, app, dirs) = start_as(stand_in);
    let store = ui.global::<Store>();
    app.open(&ui, stand_in);
    assert_eq!(store.get_need_note(), "", "nothing is said about FUSE on the store's own page");
    store.invoke_install(0);
    assert_eq!(store.get_install_state(), 2, "{}", store.get_install_message());
    let placed: Vec<String> = std::fs::read_dir(&dirs.applications)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(placed.iter().any(|name| name.ends_with(".AppImage")), "{placed:?}");
    assert!(!placed.iter().any(|name| name.ends_with(".AppDir")), "it is left as the file it is: {placed:?}");
    let entry = std::fs::read_to_string(dirs.menu.join("bilby.desktop")).unwrap();
    assert!(entry.contains(".AppImage"), "and the menu starts that file: {entry}");
    assert!(store.get_can_open(), "with Open to restart into it");
}
