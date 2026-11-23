//! Onboarding and support (F19, D11): What's New and Credits fetched from
//! the client repository on a `wiremock` GitHub, the tour's state, and the
//! support panel's issue templates.

use exilence_ce_ng_lib::github::{self, REPOSITORY};
use exilence_ce_ng_lib::links;
use ipc::command::{
    CompleteTour, GetChangelog, GetCredits, IssueTemplate, NoArgs, OpenIssueTemplate,
    OpenIssueTemplateArgs,
};
use ipc::slices::Settings;
use ipc::{IpcError, SliceName};
use tauri::async_runtime::block_on;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{Fakes, Harness, mock_server, settings_slice};

const CHANGELOG: &str = "# Changelog\n\n## 2.0.0\n\n- First release.\n";
const CREDITS: &str = "## Developers\n- CojL\n";

/// GitHub serving the client repository's `CHANGELOG.md` and `CREDITS.md`
/// on its default branch.
fn github_with_files() -> MockServer {
    block_on(async {
        let server = mock_server().await;
        for (file, body) in [(github::CHANGELOG, CHANGELOG), (github::CREDITS, CREDITS)] {
            Mock::given(method("GET"))
                .and(path(format!("/{REPOSITORY}/main/{file}")))
                .respond_with(ResponseTemplate::new(200).set_body_string(body))
                .mount(&server)
                .await;
        }
        server
    })
}

fn with_github(server: &MockServer) -> Harness {
    Harness::with(Fakes {
        github_raw_url: server.uri(),
        ..Fakes::default()
    })
}

#[test]
fn f19_changelog_and_credits_come_from_the_client_repository() {
    let github = github_with_files();
    let harness = with_github(&github);

    assert_eq!(
        harness.invoke::<GetChangelog>(&NoArgs {}).unwrap(),
        CHANGELOG
    );
    assert_eq!(harness.invoke::<GetCredits>(&NoArgs {}).unwrap(), CREDITS);
}

#[test]
fn f19_a_missing_changelog_fails_without_text() {
    // GitHub answers 404 for every file.
    let github = block_on(mock_server());
    let harness = with_github(&github);

    let error = harness.invoke::<GetChangelog>(&NoArgs {}).unwrap_err();
    assert!(matches!(error, IpcError::Internal { .. }), "{error:?}");
    // No GitHub at all.
    let error = Harness::new().invoke::<GetCredits>(&NoArgs {}).unwrap_err();
    assert!(matches!(error, IpcError::Internal { .. }), "{error:?}");
}

#[test]
fn f19_tour_completes_once_and_opens_whats_new_only_the_first_time() {
    let harness = Harness::new();
    assert!(!settings_slice(&harness).tour_completed);
    let events = harness.events(SliceName::Settings.event_name());

    assert!(harness.invoke::<CompleteTour>(&NoArgs {}).unwrap());

    let expected = Settings {
        tour_completed: true,
        whats_new_shown: true,
        ..Settings::default()
    };
    let event = events
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert_eq!(event, ipc::Event::Settings(expected.clone()));
    // A retaken tour does not open What's New again, also after a restart.
    assert!(!harness.invoke::<CompleteTour>(&NoArgs {}).unwrap());
    let harness = harness.restart();
    assert_eq!(settings_slice(&harness), expected);
    assert!(!harness.invoke::<CompleteTour>(&NoArgs {}).unwrap());
}

#[test]
fn d11_support_panel_opens_the_client_repository_issue_templates() {
    let harness = Harness::new();
    for template in [IssueTemplate::FeatureRequest, IssueTemplate::BugReport] {
        harness
            .invoke::<OpenIssueTemplate>(&OpenIssueTemplateArgs { template })
            .unwrap();
    }
    assert_eq!(
        *harness.desktop.opened.lock().unwrap(),
        [
            format!("https://github.com/{REPOSITORY}/issues/new?template=feature_request.yml"),
            format!("https://github.com/{REPOSITORY}/issues/new?template=bug_report.yml"),
        ]
    );
    assert!(links::allowed("https://github.com/exilence-ce/exilence-ce/issues/1").is_some());
}
