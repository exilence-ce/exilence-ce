//! The only place in the client that names its GitHub repository. Every GitHub
//! URL the client builds in (update files, What's New and Credits, issue
//! templates, download page) derives from [`REPOSITORY`].

use ipc::command::IssueTemplate;

/// `owner/name` of the client's GitHub repository.
pub const REPOSITORY: &str = "exilence-ce/exilence-ce";

/// The branch What's New and Credits are read from: the repository's
/// default branch (F19).
pub const BRANCH: &str = "main";

/// Where GitHub serves a repository's files as they are.
pub const RAW_URL: &str = "https://raw.githubusercontent.com";

/// What's New's markdown (F19).
pub const CHANGELOG: &str = "CHANGELOG.md";

/// The Credits' markdown (F19).
pub const CREDITS: &str = "CREDITS.md";

/// The URL of `file` on [`BRANCH`] of `repository`, served from `raw_url`
/// ([`RAW_URL`] in the app).
pub fn raw_file_url(raw_url: &str, repository: &str, file: &str) -> String {
    format!(
        "{}/{repository}/{BRANCH}/{file}",
        raw_url.trim_end_matches('/')
    )
}

/// The file of `template` in `.github/ISSUE_TEMPLATE/`.
pub fn issue_template_file(template: IssueTemplate) -> &'static str {
    match template {
        IssueTemplate::FeatureRequest => "feature_request.yml",
        IssueTemplate::BugReport => "bug_report.yml",
    }
}

/// The new-issue page of `repository` with `template` filled in (F19, D11).
pub fn issue_template_url(repository: &str, template: IssueTemplate) -> String {
    format!(
        "https://github.com/{repository}/issues/new?template={}",
        issue_template_file(template)
    )
}

/// Reads the client repository's files raw (F19, D11).
pub struct Github {
    client: reqwest::Client,
    raw_url: String,
}

impl Github {
    /// Reads from `raw_url`, [`RAW_URL`] in the app.
    pub fn new(raw_url: &str, user_agent: String) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            client: ninja::http::client(user_agent)?,
            raw_url: raw_url.to_owned(),
        })
    }

    /// The text of `file` on [`BRANCH`] of [`REPOSITORY`].
    pub async fn file(&self, file: &str) -> reqwest::Result<String> {
        let url = raw_file_url(&self.raw_url, REPOSITORY, file);
        ninja::http::send("GitHub", self.client.get(url))
            .await?
            .error_for_status()?
            .text()
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn d11_links_derive_from_the_repository_setting() {
        assert_eq!(
            raw_file_url(RAW_URL, REPOSITORY, CHANGELOG),
            "https://raw.githubusercontent.com/exilence-ce/exilence-ce/main/CHANGELOG.md"
        );
        assert_eq!(
            raw_file_url(RAW_URL, REPOSITORY, CREDITS),
            "https://raw.githubusercontent.com/exilence-ce/exilence-ce/main/CREDITS.md"
        );
        assert_eq!(
            issue_template_url(REPOSITORY, IssueTemplate::BugReport),
            "https://github.com/exilence-ce/exilence-ce/issues/new?template=bug_report.yml"
        );
        assert_eq!(
            issue_template_url(REPOSITORY, IssueTemplate::FeatureRequest),
            "https://github.com/exilence-ce/exilence-ce/issues/new?template=feature_request.yml"
        );
        assert_eq!(
            raw_file_url("http://127.0.0.1:8080/", REPOSITORY, CREDITS),
            "http://127.0.0.1:8080/exilence-ce/exilence-ce/main/CREDITS.md"
        );
    }

    #[test]
    fn d11_issue_template_links_are_allowlisted() {
        for template in [IssueTemplate::FeatureRequest, IssueTemplate::BugReport] {
            let url = issue_template_url(REPOSITORY, template);
            assert!(crate::links::allowed(&url).is_some(), "{url}");
        }
    }

    /// The templates the support panel opens exist; the bug report asks for
    /// the Diagnostics bundle (F19, F20).
    #[test]
    fn f19_issue_templates_exist_and_the_bug_report_asks_for_diagnostics() {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.github/ISSUE_TEMPLATE");
        let read = |template| {
            let path = dir.join(issue_template_file(template));
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        };
        read(IssueTemplate::FeatureRequest);
        let bug_report = read(IssueTemplate::BugReport);
        assert!(bug_report.contains("Diagnostics bundle"));
        assert!(bug_report.contains("Export diagnostics"));
    }
}
