//! Table-driven coverage of every committed environment fixture.
//!
//! `--doctor` prints a de-identified environment shape, so a user reporting a
//! Windows layout nobody on the team owns can paste that shape into
//! `tests/fixtures/environments/`. This test picks every fixture up
//! automatically: adding a file adds a case, and deleting one fails the
//! minimum-count assertion rather than quietly shrinking coverage.

use std::fs;
use std::path::{Path, PathBuf};

use zenith_lib::diagnostics::doctor::SelfCheckOutcome;
use zenith_lib::platform::path_algebra::{self, PathFlavor, ProtectedRoot};
use zenith_lib::platform::{
    EnvironmentFixture, KnownFolder, PlatformEnvironment, ProfileShape, ToolResolution,
};
use zenith_lib::signatures::SignatureRegistry;

const FIXTURE_DIR: &str = "tests/fixtures/environments";
/// The fixtures below are the documented cases; the count is asserted so a
/// removal is a deliberate, visible change.
const MINIMUM_FIXTURES: usize = 4;

fn fixture_paths() -> Vec<PathBuf> {
    let mut paths = fs::read_dir(FIXTURE_DIR)
        .unwrap_or_else(|error| panic!("{FIXTURE_DIR} must exist: {error}"))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn load(path: &Path) -> (String, EnvironmentFixture) {
    let text =
        fs::read_to_string(path).unwrap_or_else(|error| panic!("read {path:?} failed: {error}"));
    let fixture: EnvironmentFixture = serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{path:?} is not a valid environment fixture: {error}"));
    (text, fixture)
}

#[test]
fn every_committed_environment_fixture_holds_its_invariants() {
    let paths = fixture_paths();
    assert!(
        paths.len() >= MINIMUM_FIXTURES,
        "expected at least {MINIMUM_FIXTURES} committed environment fixtures, found {}",
        paths.len()
    );

    for path in &paths {
        let (text, fixture) = load(path);
        let environment = fixture.environment();
        let flavor = environment.flavor();
        let label = &fixture.name;

        // 1. The description the fixture states is the description the code
        //    sees: a fixture that cannot round-trip is describing a different
        //    machine than the test exercises.
        assert_eq!(
            environment.shape(),
            fixture.shape,
            "{label}: the fixture did not round-trip through the environment"
        );

        // 2. A fixture must not carry an account or profile path. It is meant
        //    to be pasteable into a public issue.
        for forbidden in [r"\Users\", "/Users/", "/home/", r"fixture-host"] {
            assert!(
                !text.contains(forbidden),
                "{label}: fixture text leaks a profile path ({forbidden})"
            );
        }

        // 3. Path algebra invariants hold under the stated profile and known
        //    folders, on every runner.
        let mut roots = Vec::new();
        if let Some(home) = environment.user_home() {
            roots.push(home);
        }
        for folder in KnownFolder::ALL {
            if let Some(resolved) = environment.content_dir(folder.token()) {
                roots.push(resolved);
            }
        }
        roots.push(environment.temp_dir());
        for root in &roots {
            let text = root.to_string_lossy();
            let once = path_algebra::normalize(&text, flavor);
            assert_eq!(
                once,
                path_algebra::normalize(&once, flavor),
                "{label}: normalize is not idempotent for {text:?}"
            );
            assert!(
                path_algebra::is_absolute(&text, flavor),
                "{label}: stated root {text:?} is not absolute for {flavor}"
            );
            assert!(
                !path_algebra::is_root(&text, flavor),
                "{label}: stated root {text:?} is a filesystem root"
            );
        }

        // 4. Known-folder authority: a redirected folder sits outside the
        //    profile, and the literal profile spelling must not be what the
        //    code sees.
        let home = environment.user_home().expect("fixture home");
        match environment.content_dir("documents") {
            Some(documents) => {
                let under_home = path_algebra::contains(
                    &home.to_string_lossy(),
                    &documents.to_string_lossy(),
                    flavor,
                );
                assert_eq!(
                    under_home, !fixture.shape.known_folder_redirected,
                    "{label}: known_folder_redirected disagrees with the resolved folder"
                );
            }
            None => assert!(
                !fixture.shape.known_folder_redirected,
                "{label}: a redirect is stated but no folder resolves"
            ),
        }

        // 5. Temporary-directory policy is a fact about the description, not a
        //    guess: it decides whether a temp cleanup target is inside the
        //    profile.
        let inside = path_algebra::contains(
            &home.to_string_lossy(),
            &environment.temp_dir().to_string_lossy(),
            flavor,
        );
        assert_eq!(
            inside, fixture.shape.temp_inside_profile,
            "{label}: temp_inside_profile does not match the stated roots"
        );

        // 6. Volume identity: a fixture that states no stable identifier must
        //    stay without one rather than being synthesized from the mount
        //    point.
        let stated_volumes = environment.volumes().expect("fixture states volumes");
        assert_eq!(
            stated_volumes.iter().all(|volume| volume.id.is_some()),
            fixture.shape.volume_identity,
            "{label}: volume_identity does not match the stated volumes"
        );

        // 7. A tool the fixture reports missing is an answer, not a lookup.
        for tool in &fixture.shape.missing_tools {
            assert_eq!(
                environment.tool(tool),
                Some(&ToolResolution::NotFound),
                "{label}: stated missing tool {tool} was not reported missing"
            );
        }

        // 8. Windows protection is drive-letter independent: the same fixture
        //    proves it for a `C:` machine and a `D:` machine.
        if flavor.is_windows() {
            if let Some(drive) = &fixture.shape.system_drive {
                let windows = format!(r"{drive}\Windows\System32");
                assert_eq!(
                    path_algebra::protected_root(&windows, flavor),
                    Some(ProtectedRoot::WindowsDirectory),
                    "{label}: {windows} is not protected"
                );
                let users = format!(r"{drive}\Users");
                assert_eq!(
                    path_algebra::protected_root(&users, flavor),
                    Some(ProtectedRoot::UsersRoot),
                    "{label}: {users} is not protected"
                );
                // A reviewed path below the users root stays cleanable.
                assert_eq!(
                    path_algebra::protected_root(
                        &format!(r"{drive}\Users\fixture\Downloads"),
                        flavor
                    ),
                    None,
                    "{label}: a user's own folder must stay cleanable"
                );
            }
            let profile_shape = match fixture.shape.profile_shape {
                ProfileShape::Unc => path_algebra::is_unc(&home.to_string_lossy(), flavor),
                ProfileShape::DriveRooted => !path_algebra::is_unc(&home.to_string_lossy(), flavor),
                _ => true,
            };
            assert!(
                profile_shape,
                "{label}: the stated profile shape does not match {home:?}"
            );
        }

        // 9. The shipped signature catalog must survive this environment: every
        //    resolved path stays absolute, inside its signature's scope, out of
        //    the blacklist, and declares the platform it belongs to.
        let registry = SignatureRegistry::load_embedded_with(&environment)
            .unwrap_or_else(|error| panic!("{label}: the catalog failed to load: {error}"));
        let findings = SignatureRegistry::audit_signature_platforms(&registry, &environment);
        assert!(
            findings.is_empty(),
            "{label}: the signature catalog does not hold under this environment: {findings:?}"
        );
    }
}

/// The fixture contract and the `--doctor` contract must agree. Without this
/// cross-check the two can drift into contradicting invariants: a fixture that
/// models a redirected known folder while the self-check demands that every
/// known folder live under the profile would make a healthy machine report a
/// failure.
#[test]
fn the_self_check_passes_for_every_committed_fixture() {
    for path in fixture_paths() {
        let (_, fixture) = load(&path);
        let environment = fixture.environment();
        let report = zenith_lib::diagnostics::doctor::self_check(&environment);

        let failed = report
            .checks
            .iter()
            .filter(|check| check.outcome == SelfCheckOutcome::Fail)
            .map(|check| format!("{}: {}", check.name, check.detail))
            .collect::<Vec<_>>();
        assert!(
            failed.is_empty(),
            "{}: the self-check failed for a committed environment fixture: {failed:?}",
            fixture.name
        );
    }
}

#[test]
fn a_fixture_can_describe_an_environment_the_host_is_not() {
    // The same fixture on any runner: this is what makes a user's machine a
    // permanent CI case instead of a machine nobody owns.
    let (_, fixture) = load(
        fixture_paths()
            .iter()
            .find(|path| {
                path.file_name()
                    .is_some_and(|name| name == "windows-11-d-drive-redirected-documents.json")
            })
            .expect("the redirected Windows fixture must exist"),
    );

    let environment = fixture.environment();
    assert_eq!(environment.flavor(), PathFlavor::Windows);
    assert_eq!(
        environment.expand_placeholder("~/.npm"),
        Some(PathBuf::from(r"D:\Users\fixture\.npm"))
    );
    assert_eq!(
        environment.expand_placeholder("${LOCAL_APP_DATA}/npm-cache"),
        Some(PathBuf::from(r"D:\Users\fixture\AppData\Local\npm-cache"))
    );
    assert_eq!(
        environment.content_dir("documents"),
        Some(PathBuf::from(r"D:\Redirected\Documents")),
        "the redirected known folder is the authority, not the profile spelling"
    );
    let _: PlatformEnvironment = environment;
}
