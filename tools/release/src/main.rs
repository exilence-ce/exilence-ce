//! `release-tool`, run by `.github/workflows/release.yml`:
//!
//! - `check-tag <tag>`: fails unless `<tag>` is `v` + the workspace version;
//!   prints `version=…` and `prerelease=true|false` for `$GITHUB_OUTPUT`.
//! - `notes <CHANGELOG.md> <version>`: prints the version's section.
//! - `select <releases.json>`: prints `latest <tag>` and `beta <tag>`, each
//!   only if that update file names a release.
//! - `update-file <releases.json> <tag> <signature dir>`: prints the update
//!   file naming `<tag>`, reading signatures from `<signature dir>/<asset>`.
//!
//! `releases.json` is the output of
//! `gh api --paginate repos/<owner>/<repo>/releases`.

use std::path::Path;
use std::process::ExitCode;

use release_tool::{Release, changelog_section, check_tag, parse_releases, select, update_file};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match run(&args) {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("release-tool: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[&str]) -> Result<String, String> {
    match args {
        ["check-tag", tag] => {
            let version = check_tag(tag, env!("CARGO_PKG_VERSION"))?;
            Ok(format!(
                "version={version}\nprerelease={}\n",
                !version.pre.is_empty()
            ))
        }
        ["notes", changelog, version] => {
            let changelog = read(changelog)?;
            let section = changelog_section(&changelog, version)
                .ok_or_else(|| format!("CHANGELOG.md has no section for {version}"))?;
            Ok(format!("{section}\n"))
        }
        ["select", releases] => {
            let releases = releases_from(releases)?;
            let channels = select(&releases);
            let mut output = String::new();
            for (file, release) in [("latest", channels.latest), ("beta", channels.beta)] {
                if let Some(release) = release {
                    output += &format!("{file} {}\n", release.tag_name);
                }
            }
            Ok(output)
        }
        ["update-file", releases, tag, signatures] => {
            let releases = releases_from(releases)?;
            let release = releases
                .iter()
                .find(|r| r.tag_name == *tag)
                .ok_or_else(|| format!("no release {tag}"))?;
            let file = update_file(release, |name| read(Path::new(signatures).join(name)))?;
            let json = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
            Ok(format!("{json}\n"))
        }
        _ => Err(
            "usage: release-tool check-tag <tag> | notes <changelog> <version> | \
             select <releases.json> | update-file <releases.json> <tag> <signature dir>"
                .to_owned(),
        ),
    }
}

fn read(path: impl AsRef<Path>) -> Result<String, String> {
    let path = path.as_ref();
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

fn releases_from(path: &str) -> Result<Vec<Release>, String> {
    parse_releases(&read(path)?).map_err(|e| format!("cannot parse {path}: {e}"))
}
