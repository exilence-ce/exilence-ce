//! The release workflow's logic (ADR 0003, F16): the tag check, the release
//! notes from `CHANGELOG.md`, and the update files `latest.json` and
//! `beta.json` computed from the repository's GitHub releases.

use std::collections::BTreeMap;

use semver::Version;
use serde::{Deserialize, Serialize};

/// A GitHub release as the REST API lists it; only the fields used here.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub draft: bool,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub published_at: Option<String>,
    pub assets: Vec<Asset>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Asset {
    pub name: String,
    /// `uploaded` once the upload is complete.
    pub state: String,
    pub browser_download_url: String,
}

/// An updater artifact of every release: how its asset name ends and the
/// update file platforms it serves.
pub struct Artifact {
    pub suffix: &'static str,
    pub platforms: &'static [&'static str],
    /// Only stable releases carry it.
    pub stable_only: bool,
}

/// The updater artifacts the release workflow uploads, each with its
/// signature as `<name>.sig`.
pub const ARTIFACTS: &[Artifact] = &[
    Artifact {
        suffix: "_x64-setup.exe",
        platforms: &["windows-x86_64"],
        stable_only: false,
    },
    // One universal app for both architectures.
    Artifact {
        suffix: "_universal.app.tar.gz",
        platforms: &["darwin-aarch64", "darwin-x86_64"],
        stable_only: false,
    },
    // An installed copy looks up `linux-x86_64-<bundle type>` first, then
    // `linux-x86_64`, so each Linux package names its own entry.
    Artifact {
        suffix: "_amd64.AppImage",
        platforms: &["linux-x86_64", "linux-x86_64-appimage"],
        stable_only: false,
    },
    Artifact {
        suffix: "_amd64.deb",
        platforms: &["linux-x86_64-deb"],
        stable_only: false,
    },
    // An RPM version cannot hold a prerelease (`2.0.0-beta.1`), so betas
    // ship no `.rpm`, and an `.rpm` install reads only `latest.json`.
    Artifact {
        suffix: "_x86_64.rpm",
        platforms: &["linux-x86_64-rpm"],
        stable_only: true,
    },
];

/// Installers that are not updater artifacts. A release without them is
/// still partial. The tarball is what the AUR packages install.
pub const INSTALLERS: &[&str] = &["_universal.dmg", DISTRO_TARBALL];

/// How the distro packages' tarball ends: the binary built with
/// `distro-package` in the layout of `packaging/linux/stage.sh`.
pub const DISTRO_TARBALL: &str = "_x86_64-linux.tar.gz";

/// What a signature asset's name adds to its artifact's name.
pub const SIGNATURE_SUFFIX: &str = ".sig";

/// Checks that `tag` releases `version`, the workspace version, and returns
/// it.
pub fn check_tag(tag: &str, version: &str) -> Result<Version, String> {
    let tagged = tag_version(tag).ok_or_else(|| format!("{tag} is not v<semantic version>"))?;
    if tagged.to_string() != version {
        return Err(format!(
            "{tag} does not match the workspace version {version}; bump it first"
        ));
    }
    Ok(tagged)
}

/// The version a release tag names: `v` and a semantic version.
pub fn tag_version(tag: &str) -> Option<Version> {
    Version::parse(tag.strip_prefix('v')?).ok()
}

/// The section of `version` in `changelog`, trimmed: the lines below its
/// `## <version>` heading up to the next `## ` heading. The heading may
/// write the version as `2.0.0`, `v2.0.0` or `[2.0.0]` and add text after
/// it, such as a date. `None` if there is no such section or it is empty.
pub fn changelog_section(changelog: &str, version: &str) -> Option<String> {
    let names_version = |line: &str| {
        line.strip_prefix("## ")
            .and_then(|heading| heading.split_whitespace().next())
            .map(|token| token.trim_matches(['[', ']']))
            .is_some_and(|token| token.strip_prefix('v').unwrap_or(token) == version)
    };
    let mut lines = changelog.lines().skip_while(|line| !names_version(line));
    lines.next()?;
    let section = lines
        .take_while(|line| !line.starts_with("## "))
        .collect::<Vec<_>>()
        .join("\n");
    let section = section.trim();
    (!section.is_empty()).then(|| section.to_owned())
}

/// Reads releases from the REST API's output: arrays (one per page, as
/// `gh api --paginate` prints them) or single objects, one after another.
pub fn parse_releases(json: &str) -> serde_json::Result<Vec<Release>> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Page {
        Many(Vec<Release>),
        One(Release),
    }
    let mut releases = Vec::new();
    for page in serde_json::Deserializer::from_str(json).into_iter::<Page>() {
        match page? {
            Page::Many(page) => releases.extend(page),
            Page::One(release) => releases.push(release),
        }
    }
    Ok(releases)
}

impl Release {
    /// The one fully uploaded asset whose name ends in `suffix`.
    fn uploaded(&self, suffix: &str) -> Option<&Asset> {
        let mut matching = self
            .assets
            .iter()
            .filter(|asset| asset.state == "uploaded" && asset.name.ends_with(suffix));
        let asset = matching.next()?;
        matching.next().is_none().then_some(asset)
    }

    /// The asset of `artifact` if it and its signature are fully uploaded.
    fn signed(&self, artifact: &Artifact) -> Option<&Asset> {
        let asset = self.uploaded(artifact.suffix)?;
        let signature = format!("{}{SIGNATURE_SUFFIX}", asset.name);
        self.assets
            .iter()
            .any(|a| a.state == "uploaded" && a.name == signature)
            .then_some(asset)
    }

    /// The updater artifacts this release carries: a prerelease lacks the
    /// stable-only ones.
    fn artifacts(&self) -> impl Iterator<Item = &'static Artifact> {
        let prerelease = tag_version(&self.tag_name).is_some_and(|v| !v.pre.is_empty());
        ARTIFACTS
            .iter()
            .filter(move |a| !(prerelease && a.stable_only))
    }

    /// Every installer and signed updater artifact is fully uploaded.
    pub fn is_complete(&self) -> bool {
        self.artifacts().all(|a| self.signed(a).is_some())
            && INSTALLERS.iter().all(|s| self.uploaded(s).is_some())
    }
}

/// The releases the update files name.
#[derive(Debug, Default, PartialEq)]
pub struct Channels<'a> {
    /// `latest.json`.
    pub latest: Option<&'a Release>,
    /// `beta.json`.
    pub beta: Option<&'a Release>,
}

/// Which releases the update files name (D23, ADR 0003): `latest.json` the
/// newest stable release, `beta.json` the higher of the newest stable and the
/// newest beta. Drafts, partly uploaded releases and tags that are not
/// versions (such as `updater`) are never named.
pub fn select(releases: &[Release]) -> Channels<'_> {
    let mut latest: Option<(Version, &Release)> = None;
    let mut beta: Option<(Version, &Release)> = None;
    let higher = |best: &Option<(Version, &Release)>, version: &Version| {
        best.as_ref()
            .is_none_or(|(best, _)| version.cmp_precedence(best).is_gt())
    };
    for release in releases.iter().filter(|r| !r.draft && r.is_complete()) {
        let Some(version) = tag_version(&release.tag_name) else {
            continue;
        };
        if version.pre.is_empty() && higher(&latest, &version) {
            latest = Some((version.clone(), release));
        }
        if higher(&beta, &version) {
            beta = Some((version, release));
        }
    }
    Channels {
        latest: latest.map(|(_, r)| r),
        beta: beta.map(|(_, r)| r),
    }
}

/// A Tauri static update file.
#[derive(Debug, PartialEq, Serialize)]
pub struct UpdateFile {
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pub_date: Option<String>,
    pub platforms: BTreeMap<String, Platform>,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct Platform {
    /// The content of the artifact's `.sig` asset.
    pub signature: String,
    pub url: String,
}

/// The update file naming `release`. `signature` returns the content of the
/// signature asset of the given name.
pub fn update_file(
    release: &Release,
    signature: impl Fn(&str) -> Result<String, String>,
) -> Result<UpdateFile, String> {
    let tag = &release.tag_name;
    let version = tag_version(tag).ok_or_else(|| format!("{tag} is not a version"))?;
    if release.draft || !release.is_complete() {
        return Err(format!("{tag} is a draft or partly uploaded"));
    }
    let mut platforms = BTreeMap::new();
    for artifact in release.artifacts() {
        let asset = release
            .signed(artifact)
            .ok_or_else(|| format!("{tag} lacks a signed asset"))?;
        let signature = signature(&format!("{}{SIGNATURE_SUFFIX}", asset.name))?
            .trim()
            .to_owned();
        for &platform in artifact.platforms {
            platforms.insert(
                platform.to_owned(),
                Platform {
                    signature: signature.clone(),
                    url: asset.browser_download_url.clone(),
                },
            );
        }
    }
    Ok(UpdateFile {
        version: version.to_string(),
        notes: release.body.clone().filter(|body| !body.is_empty()),
        pub_date: release.published_at.clone(),
        platforms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPO: &str = "https://github.com/owner/repo/releases/download";

    fn asset(tag: &str, name: &str) -> Asset {
        Asset {
            name: name.to_owned(),
            state: "uploaded".to_owned(),
            browser_download_url: format!("{REPO}/{tag}/{name}"),
        }
    }

    /// A published release with every installer and signed artifact it
    /// carries, as the release workflow uploads them.
    fn release(tag: &str) -> Release {
        let version = tag.trim_start_matches('v');
        let prerelease = version.contains('-');
        let mut assets = Vec::new();
        for artifact in ARTIFACTS.iter().filter(|a| !(prerelease && a.stable_only)) {
            let name = format!("Exilence-CE-NG_{version}{}", artifact.suffix);
            assets.push(asset(tag, &format!("{name}{SIGNATURE_SUFFIX}")));
            assets.push(asset(tag, &name));
        }
        for suffix in INSTALLERS {
            assets.push(asset(tag, &format!("Exilence-CE-NG_{version}{suffix}")));
        }
        Release {
            tag_name: tag.to_owned(),
            draft: false,
            body: Some(format!("Notes of {version}")),
            published_at: Some("2026-11-12T10:00:00Z".to_owned()),
            assets,
        }
    }

    fn tags<'a>(channels: &Channels<'a>) -> (Option<&'a str>, Option<&'a str>) {
        (
            channels.latest.map(|r| r.tag_name.as_str()),
            channels.beta.map(|r| r.tag_name.as_str()),
        )
    }

    #[test]
    fn d23_beta_file_names_the_higher_of_newest_stable_and_newest_beta() {
        // Before the first stable release only `beta.json` names anything.
        let releases = [release("v2.0.0-beta.1"), release("v2.0.0-beta.2")];
        assert_eq!(tags(&select(&releases)), (None, Some("v2.0.0-beta.2")));

        // A stable release above every beta: both files name it.
        let releases = [
            release("v2.0.0-beta.9"),
            release("v2.0.0"),
            release("v2.0.0-beta.10"),
        ];
        assert_eq!(tags(&select(&releases)), (Some("v2.0.0"), Some("v2.0.0")));

        // A newer beta: only `beta.json` moves.
        let releases = [
            release("v2.0.0"),
            release("v2.1.0-beta.1"),
            release("v2.0.1"),
        ];
        assert_eq!(
            tags(&select(&releases)),
            (Some("v2.0.1"), Some("v2.1.0-beta.1"))
        );

        // A stable release above that beta takes `beta.json` back.
        let releases = [release("v2.1.0-beta.1"), release("v2.1.0")];
        assert_eq!(tags(&select(&releases)), (Some("v2.1.0"), Some("v2.1.0")));
    }

    #[test]
    fn f16_drafts_partial_releases_and_other_tags_are_never_named() {
        let mut draft = release("v3.0.0");
        draft.draft = true;
        let mut uploading = release("v2.9.0");
        uploading.assets[0].state = "starter".to_owned();
        let mut unsigned = release("v2.8.0");
        unsigned
            .assets
            .retain(|a| a.name != "Exilence-CE-NG_2.8.0_amd64.AppImage.sig");
        let mut no_dmg = release("v2.7.0-beta.1");
        no_dmg.assets.retain(|a| !a.name.ends_with(".dmg"));
        let mut updater = release("updater");
        updater.assets.clear();
        let releases = [
            draft,
            uploading,
            unsigned,
            no_dmg,
            updater,
            release("not-a-version"),
            release("v2.0.0"),
        ];
        assert_eq!(tags(&select(&releases)), (Some("v2.0.0"), Some("v2.0.0")));
        assert_eq!(select(&[]), Channels::default());
    }

    #[test]
    fn f16_update_file_lists_every_platform_with_its_signature() {
        let release = release("v2.1.0");
        let file = update_file(&release, |name| Ok(format!("sig of {name}\n"))).unwrap();
        let json = serde_json::to_value(&file).unwrap();
        let url = |name: &str| format!("{REPO}/v2.1.0/Exilence-CE-NG_2.1.0{name}");
        let platform = |name: &str| {
            serde_json::json!({
                "signature": format!("sig of Exilence-CE-NG_2.1.0{name}.sig"),
                "url": url(name),
            })
        };
        assert_eq!(
            json,
            serde_json::json!({
                "version": "2.1.0",
                "notes": "Notes of 2.1.0",
                "pub_date": "2026-11-12T10:00:00Z",
                "platforms": {
                    "darwin-aarch64": platform("_universal.app.tar.gz"),
                    "darwin-x86_64": platform("_universal.app.tar.gz"),
                    "linux-x86_64": platform("_amd64.AppImage"),
                    "linux-x86_64-appimage": platform("_amd64.AppImage"),
                    "linux-x86_64-deb": platform("_amd64.deb"),
                    "linux-x86_64-rpm": platform("_x86_64.rpm"),
                    "windows-x86_64": platform("_x64-setup.exe"),
                },
            })
        );
    }

    /// RPM versions cannot hold a prerelease: a beta is complete without an
    /// `.rpm` and its update file has no RPM entry, a stable release is not.
    #[test]
    fn f16_only_stable_releases_carry_an_rpm() {
        let beta = release("v2.1.0-beta.1");
        assert!(beta.is_complete());
        let file = update_file(&beta, |_| Ok("sig".to_owned())).unwrap();
        assert!(!file.platforms.contains_key("linux-x86_64-rpm"));
        assert!(file.platforms.contains_key("linux-x86_64-deb"));

        let mut stable = release("v2.1.0");
        stable.assets.retain(|a| !a.name.contains(".rpm"));
        assert!(!stable.is_complete());
    }

    #[test]
    fn f16_update_file_refuses_partial_releases_and_missing_signatures() {
        let mut partial = release("v2.0.0");
        partial.assets.pop();
        assert!(update_file(&partial, |_| Ok(String::new())).is_err());
        let missing = update_file(&release("v2.0.0"), |name| Err(format!("{name} missing")));
        assert!(missing.unwrap_err().ends_with(".sig missing"));
    }

    #[test]
    fn parses_paginated_arrays_and_single_releases() {
        let release = |tag: &str| {
            serde_json::json!({
                "tag_name": tag, "draft": false, "body": null, "published_at": null,
                "assets": [], "prerelease": false, "id": 1,
            })
        };
        let json = format!(
            "[{}]\n[{}]\n{}",
            release("v1.0.0"),
            release("v1.1.0"),
            release("updater")
        );
        let tags: Vec<String> = parse_releases(&json)
            .unwrap()
            .into_iter()
            .map(|r| r.tag_name)
            .collect();
        assert_eq!(tags, ["v1.0.0", "v1.1.0", "updater"]);
    }

    #[test]
    fn f16_release_notes_are_the_changelog_section_of_the_version() {
        let changelog = "# Changelog\n\n\
            ## [2.0.0-beta.2] - 2026-12-01\n\n- Fix B\n\n\
            ## 2.0.0-beta.1\n\n### Added\n\n- A\n\n\
            ## v1.0.0\n- Old\n";
        assert_eq!(
            changelog_section(changelog, "2.0.0-beta.2").as_deref(),
            Some("- Fix B")
        );
        assert_eq!(
            changelog_section(changelog, "2.0.0-beta.1").as_deref(),
            Some("### Added\n\n- A")
        );
        assert_eq!(
            changelog_section(changelog, "1.0.0").as_deref(),
            Some("- Old")
        );
        // `2.0.0` is not `2.0.0-beta.1`.
        assert_eq!(changelog_section(changelog, "2.0.0"), None);
        assert_eq!(
            changelog_section("## 3.0.0\n\n## 2.0.0\n- x", "3.0.0"),
            None
        );
    }

    #[test]
    fn f16_tag_must_release_the_workspace_version() {
        assert_eq!(
            check_tag("v2.0.0-beta.1", "2.0.0-beta.1").unwrap(),
            Version::parse("2.0.0-beta.1").unwrap()
        );
        assert!(check_tag("v2.0.0", "2.0.0-beta.1").is_err());
        assert!(check_tag("2.0.0", "2.0.0").is_err());
        assert!(check_tag("vnext", "2.0.0").is_err());
    }
}
