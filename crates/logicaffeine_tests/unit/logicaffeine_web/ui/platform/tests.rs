//! The visitor's system, read from what their browser says: every row of the fixture of real user
//! agents (`tests/fixtures/visitor_systems.json`), the same table the prerendered page's inline script
//! is held to.
use super::*;

#[derive(serde::Deserialize)]
struct Row {
    name: String,
    ua_platform: Option<String>,
    platform: String,
    max_touch_points: u32,
    user_agent: String,
    system: String,
}

fn rows() -> Vec<Row> {
    serde_json::from_str(include_str!("../../../../tests/fixtures/visitor_systems.json")).expect("the fixture parses")
}

#[test]
fn every_visitor_in_the_fixture_is_read_as_their_system() {
    let rows = rows();
    assert!(rows.len() >= 20, "the fixture holds only {} visitors", rows.len());
    let wrong: Vec<String> = rows
        .iter()
        .filter_map(|row| {
            let seen = Seen { ua_platform: row.ua_platform.as_deref(), platform: &row.platform, user_agent: &row.user_agent, max_touch_points: row.max_touch_points };
            let got = system_of(&seen).name();
            (got != row.system).then(|| format!("{}: read as {got}, not {}", row.name, row.system))
        })
        .collect();
    assert!(wrong.is_empty(), "visitors read as the wrong system:\n{}", wrong.join("\n"));
}

#[test]
fn the_fixture_covers_every_system() {
    let rows = rows();
    for system in System::ALL {
        assert!(rows.iter().any(|row| row.system == system.name()), "no visitor in the fixture is on {}", system.name());
    }
}

/// Windows gets the PowerShell install, and every other system the shell one: a phone's reader pastes
/// into an agent on a Mac or a Linux machine, or into a cloud agent's Linux sandbox.
#[test]
fn only_windows_installs_with_powershell() {
    for system in System::ALL {
        let want = if *system == System::Windows { Install::Windows } else { Install::Unix };
        assert!(install_for(*system) == want, "{} installs the wrong way", system.name());
    }
}

#[test]
fn an_install_reads_back_from_its_name() {
    for install in Install::ALL {
        assert!(Install::from_name(install.name()) == Some(*install), "{} does not read back", install.name());
    }
    assert!(Install::from_name("beos").is_none());
}
