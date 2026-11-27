//! Tests of planning an install, carrying it out, starting programs and unpacking
//! AppImages. Programs are stand-ins written by the tests; nothing real is installed.

use super::*;
use crate::model::{AppId, SourceKind};

mod planning_and_installing {
    use super::*;
    use crate::asset::classify;
    use crate::http::hex;
    use sha2::{Digest, Sha256};
    use std::cell::RefCell;

    struct Fake(&'static [u8]);
    impl Http for Fake {
        fn get(&self, _: &str) -> Result<String> {
            unreachable!()
        }
        fn download(&self, _: &str, dest: &Path, progress: &mut dyn FnMut(u64, Option<u64>) -> bool) -> Result<String> {
            std::fs::create_dir_all(dest.parent().unwrap())?;
            std::fs::write(dest, self.0)?;
            assert!(progress(self.0.len() as u64, Some(self.0.len() as u64)));
            Ok(hex(&Sha256::digest(self.0)))
        }
    }

    fn asset(name: &str, digest: Option<String>) -> Asset {
        Asset { name: name.to_string(), size: 4, url: format!("https://example.invalid/{name}"), digest, info: None }
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn the_usual_browser_is_the_one_a_web_app_opens_in_when_it_can() {
        let dirs = temp_dirs("web-usual");
        let bin = dirs.cache.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        for program in ["chromium", "brave-browser", "firefox"] {
            std::fs::write(bin.join(program), "").unwrap();
        }
        let dirs = Dirs { programs: vec![bin.clone()], ..dirs };
        let lists = dirs.config.parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&lists).unwrap();
        let usual = |entry: &str| {
            let text = format!("[Added Associations]\nx-scheme-handler/https=other.desktop;\n\n[Default Applications]\ntext/html={entry}\nx-scheme-handler/https={entry};chromium.desktop;\n");
            std::fs::write(lists.join("mimeapps.list"), text).unwrap();
        };
        // Brave is the usual browser and can do it: Brave, though Chromium is here too.
        usual("brave-browser.desktop");
        assert_eq!(default_browser(&dirs).as_deref(), Some("brave-browser"));
        assert_eq!(find_app_browser(&dirs).unwrap().program, bin.join("brave-browser"));
        // Firefox is the usual browser and cannot: one that is installed and can.
        usual("firefox.desktop");
        assert_eq!(default_browser(&dirs).as_deref(), Some("firefox"));
        assert_eq!(find_app_browser(&dirs).unwrap().program, bin.join("chromium"));
        // Only Firefox here: none, and the store offers one to install.
        std::fs::remove_file(bin.join("chromium")).unwrap();
        std::fs::remove_file(bin.join("brave-browser")).unwrap();
        assert_eq!(find_app_browser(&dirs), None);
        let arch = Platform::preset("linux-x64-arch").unwrap();
        let offered = |sources: &[SourceKind]| app_browser_to_install(&arch, sources).map(|id| id.to_string());
        assert_eq!(offered(&[SourceKind::Flathub, SourceKind::Pacman]).as_deref(), Some("pacman:chromium"));
        assert_eq!(offered(&[SourceKind::Flathub]).as_deref(), Some("flathub:org.chromium.Chromium"));
        assert_eq!(offered(&[SourceKind::GitHub]), None);
        assert_eq!(app_browser_to_install(&Platform::preset("windows-x64").unwrap(), &[SourceKind::Winget]), None);
        // An odd list names no browser.
        assert_eq!(browser_in_list("[Default Applications]\nx-scheme-handler/https=../../x y.desktop\n"), None);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_website_gets_a_window_of_its_own_where_a_browser_can_give_it_one() {
        // Only Firefox, or nothing: the site opens in the usual browser, as before.
        let plain = temp_dirs("web-plain");
        assert_eq!(find_app_browser(&plain), None);
        let entry = write_web_shortcut(&plain, "web:plex.tv", "Plex", "https://plex.tv/").unwrap();
        let text = std::fs::read_to_string(entry).unwrap();
        assert!(text.contains("xdg-open\" \"https://plex.tv/\"") && !text.contains("--app="), "{text}");

        // Chromium on the PATH: the entry starts it on the site as an app, and says
        // what that window is called so the desktop gives it the site's icon.
        let mut dirs = temp_dirs("web-app");
        let bin = dirs.cache.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("chromium"), "").unwrap();
        dirs.programs = vec![bin.clone()];
        let browser = find_app_browser(&dirs).unwrap();
        assert_eq!(browser.args("https://plex.tv/"), ["--app=https://plex.tv/"]);
        let entry = write_web_shortcut(&dirs, "web:plex.tv", "Plex", "https://plex.tv/").unwrap();
        let text = std::fs::read_to_string(entry).unwrap();
        assert!(
            text.contains(&format!("Exec=\"{}\" \"--app=https://plex.tv/\"", bin.join("chromium").display())),
            "{text}"
        );
        assert!(text.contains("StartupWMClass=chrome-plex.tv__-Default\n"), "{text}");

        // Chrome as a Flatpak does as well.
        let mut dirs = temp_dirs("web-flatpak");
        std::fs::create_dir_all(dirs.flatpak_roots[0].join("app").join("com.google.Chrome")).unwrap();
        dirs.programs.clear();
        let browser = find_app_browser(&dirs).unwrap();
        assert_eq!(browser.args("https://youtube.com/"), ["run", "com.google.Chrome", "--app=https://youtube.com/"]);
        assert_eq!(browser.window_class("https://music.youtube.com/"), "chrome-music.youtube.com__-Default");
    }

    #[test]
    #[cfg(unix)]
    fn a_flathub_set_up_twice_does_not_stop_an_install() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("bilby-two-flathubs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // A Flatpak that refuses to choose an installation, as the real one does, and
        // installs when told which.
        let log = root.join("asked");
        let script = format!(
            "#!/bin/sh\necho \"$@\" >> '{}'\ncase \"$*\" in *--user*) exit 0;; esac\necho \"error: Remote 'flathub' found in multiple installations, unable to proceed in non-interactive mode\" >&2\nexit 1\n",
            log.display()
        );
        let flatpak = root.join("flatpak");
        std::fs::write(&flatpak, script).unwrap();
        std::fs::set_permissions(&flatpak, std::fs::Permissions::from_mode(0o755)).unwrap();
        let dirs = Dirs { flatpak, ..temp_dirs("two-flathubs") };
        let asset = asset("tv.plex.PlexHTPC", None);
        let plan = InstallPlan { action: Action::Flatpak, ..save_only() };
        struct NoWeb;
        impl crate::http::Http for NoWeb {
            fn get(&self, url: &str) -> Result<String> {
                Err(Error::NotFound(url.to_string()))
            }
            fn download(&self, url: &str, _: &Path, _: &mut dyn FnMut(u64, Option<u64>) -> bool) -> Result<String> {
                Err(Error::NotFound(url.to_string()))
            }
        }
        let outcome = execute(&plan, &asset, &NoWeb, &dirs, &|_| Ok(()), &mut |_, _| true).unwrap();
        assert!(outcome.message.contains("Installed from Flathub"));
        let asked = std::fs::read_to_string(&log).unwrap();
        let asked: Vec<&str> = asked.lines().collect();
        assert_eq!(asked.len(), 2, "{asked:?}");
        assert!(!asked[0].contains("--user") && asked[1].starts_with("install --user "), "{asked:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn what_the_system_looks_after_is_removed_alone_or_not_at_all() {
        let said = |id: &str, preset: &str| {
            removal(&AppId::parse(id).unwrap(), &Platform::preset(preset).unwrap()).map(|r| r.command)
        };
        assert_eq!(said("pacman:vlc", "linux-x64-arch").as_deref(), Some("sudo pacman -R --noconfirm vlc"));
        assert_eq!(said("dnf:vlc", "linux-x64-fedora").as_deref(), Some("sudo rpm -e vlc"));
        assert_eq!(said("apt:vlc", "linux-x64-debian").as_deref(), Some("sudo apt-get remove -y vlc"));
        assert_eq!(said("snap:vlc", "linux-x64-debian").as_deref(), Some("sudo snap remove vlc"));
        // Nothing the store cannot name plainly, and nothing it does not know how to remove.
        assert_eq!(said("github:kestrel-sh/kestrel", "linux-x64-debian"), None);
        let mut image = Platform::preset("linux-x64-fedora").unwrap();
        image.linux.as_mut().unwrap().image = true;
        let layered = removal(&AppId::parse("dnf:vlc").unwrap(), &image).unwrap();
        assert_eq!(layered.command, "sudo rpm-ostree uninstall vlc");
        // apt is rehearsed: what else would go is found, and stops the removal.
        let apt =
            removal(&AppId::parse("apt:python3").unwrap(), &Platform::preset("linux-x64-debian").unwrap()).unwrap();
        assert_eq!(apt.rehearsal.as_deref(), Some("apt-get -s remove python3"));
        let rehearsal = "Reading package lists...\nRemv python3 [3.11.2-1]\nRemv gnome-shell:amd64 [43.9]\nRemv ubuntu-desktop [1.5]\n";
        assert_eq!(removed_with(rehearsal, "python3"), ["gnome-shell", "ubuntu-desktop"]);
        assert!(removed_with("Remv vlc:amd64 [3.0]\n", "vlc").is_empty());
    }

    #[test]
    fn handing_over_to_another_store_opens_its_link_and_downloads_nothing() {
        let dirs = temp_dirs("handoff");
        let link = |url: &str| Asset {
            name: "Install with Steam".into(),
            size: 0,
            url: url.into(),
            digest: None,
            info: Some(AssetInfo {
                format: Format::StoreApp,
                os: crate::asset::TargetOs::Known(crate::platform::Os::Linux),
                arch: crate::asset::TargetArch::Unknown,
                libc: None,
                tv: false,
                debug: false,
            }),
        };
        let platform = Platform::preset("linux-x64-debian").unwrap();
        let asset = link("steam://install/620");
        let made = plan(&asset, asset.info.as_ref().unwrap(), &platform);
        assert!(made.action == Action::OpenStore && made.verb == "Install with Steam");
        let opened = RefCell::new(Vec::new());
        let open = |path: &Path| {
            opened.borrow_mut().push(path.to_path_buf());
            Ok(())
        };
        let outcome = execute(&made, &asset, &Fake(b""), &dirs, &open, &mut |_, _| true).unwrap();
        assert_eq!(opened.borrow()[0].to_str(), Some("steam://install/620"));
        assert!(outcome.file.is_none() && outcome.message.starts_with("Steam is taking it from here"));
        assert!(!dirs.downloads.exists(), "nothing was downloaded");

        // Only a store's own link followed by a number is ever opened.
        for bad in [
            "steam://install/620/../x",
            "steam://run/620",
            "file:///etc/passwd",
            "https://example.com/app/620",
            "steam://install/",
        ] {
            assert!(!store_link_ok(bad), "{bad}");
            let asset = link(bad);
            assert!(execute(&made, &asset, &Fake(b""), &dirs, &open, &mut |_, _| true).is_err(), "{bad}");
        }
        assert!(store_link_ok("macappstore://apps.apple.com/app/id6746662575"));
        assert_eq!(opened.borrow().len(), 1);
        let mac = link("macappstore://apps.apple.com/app/id1");
        assert_eq!(plan(&mac, mac.info.as_ref().unwrap(), &platform).verb, "Get on the App Store");
        // The Microsoft Store: its own scheme and a product id, nothing else after it.
        assert!(store_link_ok("ms-windows-store://pdp/?ProductId=9NCBCSZSJRSB"));
        assert!(store_link_ok("https://apps.microsoft.com/detail/XPDC2RH70K22MN"));
        assert!(store_link_ok("https://www.gog.com/en/game/the_witcher_3_wild_hunt_game"));
        assert!(!store_link_ok("https://www.gog.com/en/game/../../account"));
        assert!(!store_link_ok("https://www.gog.com/en/game/x?redirect=elsewhere"));
        assert!(store_link_ok("https://play.google.com/store/apps/details?id=org.mozilla.firefox"));
        assert!(!store_link_ok("https://play.google.com/store/apps/details?id=org.mozilla.firefox&referrer=x"));
        assert!(!store_link_ok("https://play.google.com/store/search?q=firefox"));
        for bad in [
            "ms-windows-store://pdp/?ProductId=",
            "ms-windows-store://pdp/?ProductId=9NCBCSZSJRSB&mode=mini",
            "ms-windows-store://pdp/?ProductId=9ncbcszsjrsb",
            "ms-windows-store://navigatetopage/?Id=Gaming",
            "ms-settings:privacy",
        ] {
            assert!(!store_link_ok(bad), "{bad}");
        }
        let windows = link("ms-windows-store://pdp/?ProductId=9NCBCSZSJRSB");
        assert_eq!(plan(&windows, windows.info.as_ref().unwrap(), &platform).verb, "Get from the Microsoft Store");
    }

    #[test]
    fn the_store_puts_itself_in_the_app_menu_once_and_follows_its_file() {
        let dirs = temp_dirs("self");
        let program = Path::new("/home/someone/My Apps/bilby-v0.1.3-x86_64.AppImage");
        assert!(register_self(&dirs, program, b"png").unwrap(), "the first run writes the entry");
        let entry = std::fs::read_to_string(self_entry_path(&dirs)).unwrap();
        assert!(entry.contains("\nName=Bilby\n"));
        assert!(entry.contains("\nExec=\"/home/someone/My Apps/bilby-v0.1.3-x86_64.AppImage\"\n"), "{entry}");
        // Joined piece by piece, as the code does, so the separators agree on every system.
        let icons = ["icons", "hicolor", "512x512", "apps"]
            .iter()
            .fold(dirs.menu.parent().unwrap().to_path_buf(), |path, part| path.join(part));
        let icon_named_in = |entry: &str| -> PathBuf {
            PathBuf::from(entry.lines().find_map(|line| line.strip_prefix("Icon=")).unwrap())
        };
        let icon = icon_named_in(&entry);
        assert_eq!(icon.parent(), Some(icons.as_path()), "{entry}");
        assert_eq!(std::fs::read(&icon).unwrap(), b"png");
        assert_eq!(std::fs::read(icons.join("bilby.png")).unwrap(), b"png", "found by name too, for notifications");

        // A new logo goes in a file with a new name, because a launcher that has shown
        // the old file goes on showing what it remembers of it. The old file is removed.
        assert!(register_self(&dirs, program, b"new logo").unwrap());
        let newer = icon_named_in(&std::fs::read_to_string(self_entry_path(&dirs)).unwrap());
        assert_ne!(newer, icon);
        assert_eq!(std::fs::read(&newer).unwrap(), b"new logo");
        assert_eq!(std::fs::read(icons.join("bilby.png")).unwrap(), b"new logo");
        assert!(!icon.exists(), "the earlier logo is not left behind");
        assert!(register_self(&dirs, program, b"png").unwrap());

        // Starting again changes nothing; starting from somewhere else updates it.
        assert!(!register_self(&dirs, program, b"png").unwrap());
        assert!(register_self(&dirs, Path::new("/opt/bilby.AppImage"), b"png").unwrap());
        assert!(std::fs::read_to_string(self_entry_path(&dirs)).unwrap().contains("Exec=\"/opt/bilby.AppImage\""));

        // Only its own entry is ever removed.
        unregister_self(&dirs);
        assert!(!self_entry_path(&dirs).exists());
        std::fs::write(self_entry_path(&dirs), "[Desktop Entry]\nName=Someone else's\n").unwrap();
        unregister_self(&dirs);
        assert!(self_entry_path(&dirs).exists());
    }

    fn temp_dirs(tag: &str) -> Dirs {
        let root = std::env::temp_dir().join(format!("bilby-install-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        Dirs {
            downloads: root.join("dl"),
            applications: root.join("apps"),
            menu: root.join("menu"),
            flatpak: root.join("flatpak-program"),
            elevate: root.join("elevate-program"),
            terminal: None,
            flatpak_roots: vec![root.join("flatpak")],
            programs: Vec::new(),
            cache: root.join("cache"),
            config: root.join("config"),
            desktop: None,
        }
    }

    fn plan_for(name: &str, preset: &str) -> InstallPlan {
        plan(&asset(name, None), &classify(name).unwrap(), &Platform::preset(preset).unwrap())
    }

    #[test]
    #[cfg(unix)]
    fn a_windows_installer_is_set_up_on_linux_and_removed_with_everything_it_brought() {
        use std::os::unix::fs::PermissionsExt;
        let mut linux = Platform::preset("linux-x64-debian").unwrap();
        let exe = asset("thing-setup-x64.exe", None);
        let info = classify(&exe.name).unwrap();
        // Where Windows programs are not offered, an installer is only a file to keep.
        assert_ne!(plan(&exe, &info, &linux).action, Action::WindowsApp);
        linux.linux.as_mut().unwrap().windows = true;
        let made = plan(&exe, &info, &linux);
        assert_eq!((&made.action, made.verb.as_str()), (&Action::WindowsApp, "Install"));
        assert!(made.steps.iter().any(|step| step.contains("700 MB")), "{:?}", made.steps);
        assert!(!unattended(&made) && saves_a_file(&made.action));

        // A stand-in for the launcher: its "install" makes the app.
        let mut dirs = temp_dirs("windows-app");
        let bin = dirs.cache.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let fake = bin.join("umu-run");
        let body = "#!/bin/sh\nmkdir -p \"$WINEPREFIX/drive_c/Program Files/Thing\"\nprintf app > \"$WINEPREFIX/drive_c/Program Files/Thing/Thing.exe\"\n";
        std::fs::write(&fake, body).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        dirs.programs = vec![bin];
        let open = |_: &Path| -> Result<()> { unreachable!("nothing is opened: the store runs the installer itself") };
        let outcome = execute(&made, &exe, &Fake(b"MZ..installer"), &dirs, &open, &mut |_, _| true).unwrap();
        let launcher = outcome.file.clone().unwrap();
        assert!(outcome.store_managed && outcome.message.contains("Proton"), "{}", outcome.message);
        assert!(launcher.starts_with(&dirs.applications) && launcher.is_file());
        assert!(!dirs.downloads.join("thing-setup-x64.exe").exists(), "the installer is not left in Downloads");
        // Its entry in the app menu starts the launcher, and asking for its picture does not.
        let entry = add_menu_entry(&dirs, "github:x/thing", "Thing", &launcher).unwrap();
        let text = std::fs::read_to_string(&entry).unwrap();
        assert!(text.contains(&format!("Exec=\"{}\"", launcher.display())) && !text.contains("Icon="), "{text}");
        // Removing it removes its whole folder, and nothing outside it.
        let folder = launcher.parent().unwrap().to_path_buf();
        assert!(folder.join("prefix").join("drive_c").is_dir());
        assert!(remove_placed(&dirs, &launcher));
        assert!(!folder.exists() && crate::winrun::root(&dirs).is_dir());
    }

    #[test]
    fn plans_match_the_format() {
        assert_eq!(
            plan_for("a_1.0_amd64.deb", "linux-x64-debian").command.as_deref(),
            Some("sudo apt install ./a_1.0_amd64.deb")
        );
        assert_eq!(
            plan_for("a-1.0.x86_64.rpm", "linux-x64-fedora").command.as_deref(),
            Some("sudo dnf install ./a-1.0.x86_64.rpm")
        );
        assert_eq!(plan_for("A-x86_64.AppImage", "linux-x64-fedora").action, Action::PlaceAppImage);
        assert_eq!(plan_for("a-arm64-v8a.apk", "android-arm64").action, Action::AndroidInstaller);
        assert_eq!(plan_for("a-x64.msi", "windows-x64").action, Action::OpenFile);
        assert_eq!(plan_for("a-linux-x86_64.tar.gz", "linux-x64-debian").verb, "Download");
    }

    #[test]
    fn only_https_links_are_opened() {
        for bad in
            ["file:///etc/passwd", "javascript:alert(1)", "http://example.com", "", "https://a b", "/bin/sh", "-rf"]
        {
            assert!(open_url(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn repository_packages_are_left_to_the_package_manager() {
        use crate::asset::{TargetArch, TargetOs};
        use crate::platform::Os;
        let info = AssetInfo {
            format: Format::RepoPackage,
            os: TargetOs::Known(Os::Linux),
            arch: TargetArch::Any,
            libc: None,
            tv: false,
            debug: false,
        };
        let a = asset("linux-cachyos (cachyos-v3)", None);
        let arch = plan(&a, &info, &Platform::preset("linux-x64-arch").unwrap());
        assert_eq!(arch.action, Action::RunCommand);
        assert_eq!(arch.command.as_deref(), Some("sudo pacman -S --noconfirm --needed linux-cachyos"));
        assert_eq!(arch.verb, "Install");
        let on = |preset: &str| {
            plan(&asset("ripgrep (repo)", None), &info, &Platform::preset(preset).unwrap()).command.unwrap()
        };
        assert_eq!(on("linux-x64-debian"), "sudo apt-get install -y ripgrep");
        assert_eq!(on("linux-x64-fedora"), "sudo dnf install -y ripgrep");

        // The store runs it: "sudo" becomes the desktop's own password prompt, and
        // nothing is downloaded by the store itself.
        let dirs = temp_dirs("repo");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::create_dir_all(dirs.elevate.parent().unwrap()).unwrap();
            let log = dirs.elevate.with_extension("log");
            let script = |code: u8| {
                format!("#!/bin/sh\necho \"$@\" >> '{}'\necho 'it said why' >&2\nexit {code}\n", log.display())
            };
            std::fs::write(&dirs.elevate, script(0)).unwrap();
            std::fs::set_permissions(&dirs.elevate, std::fs::Permissions::from_mode(0o755)).unwrap();
            let out = execute(&arch, &a, &Fake(b""), &dirs, &|_| unreachable!(), &mut |_, _| true).unwrap();
            assert_eq!(out.message, "Installed.");
            assert_eq!(std::fs::read_to_string(&log).unwrap().trim(), "pacman -S --noconfirm --needed linux-cachyos");
            // A failure passes on what the package manager said.
            std::fs::write(&dirs.elevate, script(1)).unwrap();
            let failed = execute(&arch, &a, &Fake(b""), &dirs, &|_| unreachable!(), &mut |_, _| true).unwrap_err();
            assert_eq!(failed.to_string(), "pacman could not do it: it said why");
        }
        // Without a way to ask for the password, it says so instead of pretending.
        let nowhere = Dirs { elevate: dirs.elevate.with_extension("missing"), ..temp_dirs("repo2") };
        let failed = execute(&arch, &a, &Fake(b""), &nowhere, &|_| unreachable!(), &mut |_, _| true).unwrap_err();
        assert!(failed.to_string().contains("could not be started"), "{failed}");
        assert!(!dirs.downloads.exists());
    }

    #[test]
    fn file_names_cannot_escape() {
        assert_eq!(safe_file_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(safe_file_name("..\\evil.exe"), "_evil.exe");
        assert_eq!(safe_file_name(".."), "download");
        assert_eq!(safe_file_name("Thing-1.0.AppImage"), "Thing-1.0.AppImage");
    }

    #[test]
    fn appimage_is_verified_made_executable_and_moved() {
        let dirs = temp_dirs("appimage");
        let good = format!("sha256:{}", hex(&Sha256::digest(b"data")));
        let a = asset("Thing-x86_64.AppImage", Some(good));
        let p = plan_for("Thing-x86_64.AppImage", "linux-x64-fedora");
        let out = execute(&p, &a, &Fake(b"data"), &dirs, &|_| Ok(()), &mut |_, _| true).unwrap();
        let file = out.file.unwrap();
        assert_eq!(file, dirs.applications.join("Thing-x86_64.AppImage"));
        assert!(out.message.starts_with("Checksum verified."));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o111, 0o111);
        }
    }

    #[test]
    fn only_a_digest_that_can_be_checked_counts_as_one() {
        let dirs = temp_dirs("digest-kinds");
        let p = plan_for("a-linux-x86_64.tar.gz", "linux-x64-debian");
        for odd in ["sha512:abcdef", "md5:900150983cd24fb0d6963f7d28e17f72", "sha256:", "sha256:xyz", "garbage"] {
            let a = asset("a-linux-x86_64.tar.gz", Some(odd.to_string()));
            assert_eq!(checkable_digest(&a), None, "{odd}");
            let planned = plan(&a, &classify(&a.name).unwrap(), &Platform::preset("linux-x64-debian").unwrap());
            assert!(!planned.steps.iter().any(|s| s.contains("SHA-256")), "{odd}: no check is promised");
            let out = execute(&p, &a, &Fake(b"data"), &dirs, &|_| Ok(()), &mut |_, _| true).unwrap();
            assert!(!out.message.contains("verified"), "{odd}: no check is claimed: {}", out.message);
        }
        // Upper-case hex is the same digest.
        let upper = format!("sha256:{}", hex(&Sha256::digest(b"data")).to_uppercase());
        let a = asset("a-linux-x86_64.tar.gz", Some(upper));
        let out = execute(&p, &a, &Fake(b"data"), &dirs, &|_| Ok(()), &mut |_, _| true).unwrap();
        assert!(out.message.starts_with("Checksum verified."));
    }

    #[test]
    fn tampered_download_is_rejected_and_deleted() {
        let dirs = temp_dirs("tamper");
        let a = asset("a_1.0_amd64.deb", Some(format!("sha256:{}", "0".repeat(64))));
        let p = plan_for("a_1.0_amd64.deb", "linux-x64-debian");
        let opened = RefCell::new(Vec::new());
        let err = execute(
            &p,
            &a,
            &Fake(b"data"),
            &dirs,
            &|path| {
                opened.borrow_mut().push(path.to_path_buf());
                Ok(())
            },
            &mut |_, _| true,
        )
        .unwrap_err();
        assert!(matches!(err, Error::DigestMismatch { .. }));
        assert!(opened.borrow().is_empty(), "a file that fails verification is never opened");
        assert!(!dirs.downloads.join("a_1.0_amd64.deb").exists());
    }

    #[test]
    fn native_packages_are_handed_to_the_system() {
        let dirs = temp_dirs("deb");
        let a = asset("a_1.0_amd64.deb", None);
        let p = plan_for("a_1.0_amd64.deb", "linux-x64-debian");
        let opened = RefCell::new(Vec::new());
        execute(
            &p,
            &a,
            &Fake(b"data"),
            &dirs,
            &|path| {
                opened.borrow_mut().push(path.to_path_buf());
                Ok(())
            },
            &mut |_, _| true,
        )
        .unwrap();
        assert_eq!(*opened.borrow(), vec![dirs.downloads.join("a_1.0_amd64.deb")]);
    }

    #[test]
    fn an_appimage_gets_one_menu_entry_that_cannot_be_broken_out_of() {
        let dirs = temp_dirs("menu");
        let file = dirs.applications.join("My $HOME `app` \"x\" 100%.AppImage");
        let path = add_menu_entry(&dirs, "github:Owner/Thing", "Thing\nExec=evil", &file).unwrap();
        assert_eq!(path, dirs.menu.join("bilby-github-owner-thing.desktop"));
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().filter(|l| l.starts_with("Exec=")).count(), 1, "a name cannot add keys: {text}");
        assert!(text.contains("Name=ThingExec=evil\n"));
        let exec = text.lines().find(|l| l.starts_with("Exec=")).unwrap();
        assert!(exec.contains(r#"\\$HOME"#) && exec.contains(r#"\\`app\\`"#) && exec.contains(r#"\\"x\\""#), "{exec}");
        assert!(exec.contains("100%%") && exec.ends_with("\" %U"), "{exec}");

        // A new version of the same app replaces the entry instead of adding one.
        add_menu_entry(&dirs, "github:Owner/Thing", "Thing", &dirs.applications.join("Thing-2.AppImage")).unwrap();
        assert_eq!(std::fs::read_dir(&dirs.menu).unwrap().count(), 1);
        assert!(std::fs::read_to_string(&path).unwrap().contains("Thing-2.AppImage"));

        // Removing only ever touches entries the store wrote.
        let foreign = menu_entry_path(&dirs, "github:someone/else");
        std::fs::write(&foreign, "[Desktop Entry]\nName=Not ours\n").unwrap();
        remove_menu_entry(&dirs, "github:someone/else");
        assert!(foreign.exists());
        remove_menu_entry(&dirs, "github:Owner/Thing");
        assert!(!path.exists());
    }

    #[test]
    #[cfg(unix)]
    fn what_a_failed_command_printed_is_kept_for_whoever_asks() {
        let (result, lines) = journal::keep(|| {
            run_program(
                Path::new("sh"),
                &["-c", "echo unpacking; printf 'half\\rdone\\n'; echo 'no space left' >&2; exit 3"],
                "The installer",
                &mut |_, _| true,
            )
        });
        // The person is told the program's own last word, as before.
        assert_eq!(result.unwrap_err().to_string(), "The installer could not do it: no space left");
        // And everything else is there to be read: what was run, both things it wrote
        // to, and how it ended. A line it drew over ("half") is not something it said.
        let all = lines.join("\n");
        assert!(lines[0].starts_with("Running sh -c echo unpacking"), "{all}");
        assert!(all.contains("It printed:\n  unpacking\n  done\n"), "{all}");
        assert!(all.contains("It reported:\n  no space left\n"), "{all}");
        assert!(all.contains("It ended with exit status: 3"), "{all}");
        assert!(!all.contains("\n  half"), "{all}");

        // A command that cannot be started at all says so too.
        let (result, lines) = journal::keep(|| {
            let plan = InstallPlan {
                command: Some("bilby-no-such-program --now".to_string()),
                ..plan_for("x.AppImage", "linux-x64-debian")
            };
            let plan = InstallPlan { action: Action::RunCommand, ..plan };
            let dirs = temp_dirs("journal");
            execute(&plan, &asset("thing", None), &Fake(b""), &dirs, &|_| Ok(()), &mut |_, _| true)
        });
        assert!(result.is_err());
        assert!(lines.iter().any(|line| line.starts_with("Running bilby-no-such-program --now")), "{lines:?}");
        assert!(lines.last().unwrap().starts_with("Failed: "), "{lines:?}");
    }
}

mod launch_tests {
    use super::*;

    #[test]
    fn how_far_a_flatpak_install_has_got_is_read_from_what_it_prints() {
        assert_eq!(flatpak_progress("Installing 1/5…"), Some(0));
        assert_eq!(flatpak_progress("Installing 1/5… ███████████▍         47%  30.9 MB/s"), Some(94));
        assert_eq!(flatpak_progress("Installing 4/5… ██████████▌           53%  30.9 MB/s"), Some(706));
        assert_eq!(flatpak_progress("Updating 2/2… ████████████████████ 100%  1.0 MB/s"), Some(990));
        assert_eq!(flatpak_progress("Installing 1/1… ████████████████████ 100%"), Some(990));
        assert_eq!(flatpak_progress("Looking for matches…"), None);
        assert_eq!(flatpak_progress("Installing runtime/org.freedesktop.Platform/x86_64/23.08"), None);
        assert_eq!(flatpak_progress(" 1.\t   \ttv.plex.PlexHTPC\tstable\ti\tflathub\t< 150.7 MB"), None);
    }

    #[test]
    fn a_program_that_closes_at_once_gives_its_reason() {
        assert!(
            why_it_closed("dlopen(): error loading libfuse.so.2\nAppImages require FUSE to run.").contains("libfuse2")
        );
        assert!(why_it_closed("FATAL:setuid_sandbox_host.cc(163)] The SUID sandbox helper binary was found, but is not configured correctly.").contains("sandbox"));
        let missing = why_it_closed(
            "app: error while loading shared libraries: libwebkit2gtk-4.1.so.0: cannot open shared object file",
        );
        assert!(missing.contains("libwebkit2gtk-4.1.so.0"), "{missing}");
        assert_eq!(why_it_closed("first\nsecond line\n\n"), "It closed straight away. It said: second line");
        assert_eq!(why_it_closed(""), "It closed straight away without saying why.");
    }

    #[cfg(unix)]
    #[test]
    fn starting_is_watched_for_a_moment() {
        use std::os::unix::fs::PermissionsExt;
        let _one_at_a_time = super::SCRIPTS.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("bilby-launch-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = |name: &str, body: &str| {
            let path = dir.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        };
        let watch = std::time::Duration::from_millis(1500);
        let broken = script("broken", "echo 'AppImages require FUSE to run.' >&2; exit 127");
        assert!(launch_watched(&broken, &[], watch).unwrap_err().to_string().contains("FUSE"));
        // One that stays up, and one that finishes cleanly, both count as started.
        assert!(launch_watched(&script("stays", "sleep 3"), &[], std::time::Duration::from_millis(300)).is_ok());
        assert!(launch_watched(&script("done", "exit 0"), &[], watch).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Tests that write a script and start it take turns. A script still open for writing
/// in one test, when another test starts a program, cannot be started ("text file
/// busy"): the new process holds the open file for a moment.
#[cfg(unix)]
static SCRIPTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(unix)]
mod unpack_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A stand-in AppImage: a script that unpacks itself the way a real one does.
    fn stand_in(dir: &Path, name: &str, works: bool) -> PathBuf {
        let body = if works {
            "#!/bin/sh\nif [ \"$1\" = \"--appimage-extract\" ]; then mkdir -p squashfs-root/usr/share; printf '#!/bin/sh\\necho started\\n' > squashfs-root/AppRun; chmod 644 squashfs-root/AppRun; printf '\\211PNG....' > squashfs-root/usr/share/app.png; ln -s usr/share/app.png squashfs-root/.DirIcon; exit 0; fi\nexit 1\n"
        } else {
            "#!/bin/sh\nexit 1\n"
        };
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn a_program_knows_where_it_lives_and_is_not_told_by_whoever_started_it() {
        let root = std::env::temp_dir().join(format!("bilby-own-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let old = root.join("Downloads").join("bilby-old.AppImage");
        let mounted = root.join("mount_old");
        let unpacked = root.join("Applications").join("bilby-new.AppDir");
        for folder in [old.parent().unwrap(), &mounted.join("usr/bin"), &unpacked.join("usr/bin")] {
            std::fs::create_dir_all(folder).unwrap();
        }
        for file in [&old, &mounted.join("usr/bin/bilby"), &unpacked.join("usr/bin/bilby"), &unpacked.join("AppRun")] {
            std::fs::write(file, b"x").unwrap();
        }
        // Running out of the AppImage that is named: that file.
        assert_eq!(own_program(&mounted.join("usr/bin/bilby"), Some(&old), Some(&mounted)), Some(old.clone()));
        // An unpacked copy started by the old AppImage, which left its own name behind:
        // it lives in its folder, not in the file that started it.
        let exe = unpacked.join("usr/bin/bilby");
        assert_eq!(own_program(&exe, Some(&old), Some(&mounted)), Some(unpacked.join("AppRun")));
        assert_eq!(own_program(&exe, None, None), Some(unpacked.join("AppRun")));
        // A plain program somewhere: nowhere in particular.
        assert_eq!(own_program(&root.join("bin/bilby"), None, None), None);
        assert_eq!(own_program(&root.join("bin/bilby"), Some(&old), Some(&mounted)), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn what_is_started_is_not_told_it_is_the_stores_appimage() {
        let _one_at_a_time = SCRIPTS.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("bilby-env-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("says");
        std::fs::write(&script, format!("#!/bin/sh\nenv > '{}/env'\n", dir.display())).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut command = std::process::Command::new(&script);
        command.env("APPIMAGE", "/somewhere/store.AppImage").env("APPDIR", "/tmp/.mount_store").env("KEPT", "yes");
        not_ours(&mut command);
        assert!(not_while_busy(|| command.status()).unwrap().success());
        let env = std::fs::read_to_string(dir.join("env")).unwrap();
        assert!(!env.contains("APPIMAGE=") && !env.contains("APPDIR="), "{env}");
        assert!(env.contains("KEPT=yes"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_appimage_is_unpacked_into_a_folder_that_runs_without_fuse() {
        let _one_at_a_time = SCRIPTS.lock().unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join(format!("bilby-unpack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dirs = Dirs {
            applications: root.join("Applications"),
            cache: root.join("cache"),
            menu: root.join("menu"),
            ..Dirs::for_current_user()
        };
        std::fs::create_dir_all(&dirs.applications).unwrap();
        let appimage = stand_in(&dirs.applications, "Gauge-2.1.0-x86_64.AppImage", true);

        let program = unpack_appimage(&appimage).unwrap();
        assert_eq!(program, dirs.applications.join("Gauge-2.1.0-x86_64.AppDir").join("AppRun"));
        assert!(program.is_file() && !appimage.exists(), "the folder takes the file's place");
        assert_ne!(std::fs::metadata(&program).unwrap().permissions().mode() & 0o111, 0, "and can be started");
        assert!(!dirs.applications.join(".unpacking-Gauge-2.1.0-x86_64").exists(), "nothing is left lying about");
        assert_eq!(unpacked_folder(&program), Some(dirs.applications.join("Gauge-2.1.0-x86_64.AppDir")));
        assert_eq!(unpacked_folder(&dirs.applications.join("x.AppImage")), None);
        // It starts like any program.
        assert!(launch_watched(&program, &[], std::time::Duration::from_millis(800)).is_ok());

        // Its menu entry starts the folder's program and carries its icon.
        let entry = add_menu_entry(&dirs, "own:workshop/gauge", "Gauge", &program).unwrap();
        let text = std::fs::read_to_string(entry).unwrap();
        assert!(text.contains("Gauge-2.1.0-x86_64.AppDir/AppRun"), "{text}");
        // Icons are taken out of the app only on Linux, where menu entries are used.
        assert!(text.contains("Icon=") || !cfg!(target_os = "linux"), "{text}");

        // Removing it removes the whole folder, and nothing outside Applications.
        assert!(!remove_placed(&dirs, &root.join("elsewhere").join("AppRun")));
        assert!(remove_placed(&dirs, &program));
        assert!(!dirs.applications.join("Gauge-2.1.0-x86_64.AppDir").exists());

        // One that cannot unpack itself is left as it was, with the reason.
        let stubborn = stand_in(&dirs.applications, "Old.AppImage", false);
        assert!(unpack_appimage(&stubborn).unwrap_err().to_string().contains("could not be unpacked"));
        assert!(stubborn.exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(unix)]
mod unpack_for_real {
    /// Not a check that runs by itself: unpacks a real AppImage named in
    /// `BILBY_APPIMAGE` and prints the program to start.
    #[test]
    #[ignore]
    fn unpack_the_named_appimage() {
        let Ok(path) = std::env::var("BILBY_APPIMAGE") else { return };
        let program = super::unpack_appimage(std::path::Path::new(&path)).unwrap();
        println!("PROGRAM {}", program.display());
    }
}
