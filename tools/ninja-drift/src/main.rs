//! `ninja-drift`, the poe.ninja drift check (parity-verification.md, CI), run
//! weekly by `.github/workflows/ninja-drift.yml`: fetches poe.ninja's price
//! leagues, then every price request of the current challenge league and of
//! Standard through the app's poe.ninja client ([`ninja`]), and prices the
//! fixture stash (`fixtures/pricing/`) with each price table.
//!
//! Usage: `ninja-drift [<base URL>]`, poe.ninja by default.
//!
//! Exit status:
//! - 0: every answer parsed and the fixture stash has a value in each league.
//! - 1: drift, the workflow opens an issue: an answer does not parse, poe.ninja
//!   answers a request with a client error other than 404 (no prices),
//!   Standard is not a price league, or the fixture stash prices to 0.
//! - 2: the check could not run: poe.ninja unreachable, 429 or a server error.

#[path = "../../../fixtures/pricing/snapshot.rs"]
mod snapshot;

use std::error::Error as _;
use std::path::Path;
use std::process::ExitCode;

use domain::pricing::{PriceTable, PricingOptions};
use ninja::{Ninja, Overviews};
use snapshot::Snapshot;

const USER_AGENT: &str = concat!(
    "exilence-ce-ng-ninja-drift/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/exilence-ce/exilence-ce)"
);

const STANDARD: &str = "Standard";

/// Why the check failed.
enum Failure {
    /// poe.ninja changed: exit status 1.
    Drift(String),
    /// The check could not run: exit status 2.
    NotRun(String),
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let base_url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| ninja::API_URL.to_owned());
    match check(&base_url).await {
        Ok(()) => {
            println!("ninja-drift: poe.ninja parses and prices the fixture stash");
            ExitCode::SUCCESS
        }
        Err(Failure::Drift(message)) => {
            eprintln!("ninja-drift: drift: {message}");
            ExitCode::from(1)
        }
        Err(Failure::NotRun(message)) => {
            eprintln!("ninja-drift: check not run: {message}");
            ExitCode::from(2)
        }
    }
}

async fn check(base_url: &str) -> Result<(), Failure> {
    let ninja = Ninja::new(base_url, USER_AGENT.to_owned())
        .map_err(|e| Failure::NotRun(format!("no poe.ninja client for {base_url}: {e}")))?;
    let leagues = ninja
        .price_leagues()
        .await
        .map_err(|e| failure("the price leagues", &e))?;
    println!("price leagues: {}", leagues.join(", "));
    if !leagues.iter().any(|l| l == STANDARD) {
        return Err(Failure::Drift(format!(
            "{STANDARD} is not among the price leagues"
        )));
    }
    let mut checked = Vec::new();
    match challenge_league(&leagues) {
        Some(league) => checked.push(league),
        // Between leagues poe.ninja prices only the permanent ones.
        None => println!("::warning::no challenge league among the price leagues"),
    }
    checked.push(STANDARD);

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pricing");
    let snapshot = Snapshot::load(&root);
    let tabs = snapshot.tabs();
    for league in checked {
        let entries = ninja
            .price_table(league, &mut Overviews::default())
            .await
            .map_err(|e| failure(&format!("the price table of {league}"), &e))?;
        let table = PriceTable::new(&entries, &PricingOptions::default(), &[]);
        let value = table.price_tabs(&tabs).total_value();
        println!(
            "{league}: {} prices, fixture stash worth {value:.1} chaos",
            entries.len()
        );
        if value <= 0.0 {
            return Err(Failure::Drift(format!(
                "the fixture stash prices to 0 in {league} ({} prices)",
                entries.len()
            )));
        }
    }
    Ok(())
}

/// The current challenge league: the first price league, in poe.ninja's
/// order, that is not Standard and not a Hardcore, Ruthless or SSF league.
/// poe.ninja lists the challenge league first (`Mercenaries`, `Hardcore
/// Mercenaries`, `Standard`, `Hardcore`); an event league listed before it
/// would be checked instead. None between leagues.
fn challenge_league(leagues: &[String]) -> Option<&str> {
    leagues.iter().map(String::as_str).find(|league| {
        *league != STANDARD
            && !["Hardcore", "Ruthless", "SSF"]
                .iter()
                .any(|word| league.contains(word))
    })
}

/// `error` while fetching `what`, with the causes its message lacks: drift
/// unless poe.ninja was unreachable, rate limited or failing.
fn failure(what: &str, error: &ninja::Error) -> Failure {
    let mut message = format!("{what}: {error}");
    let mut source = error.source();
    while let Some(cause) = source {
        let cause_message = cause.to_string();
        if !message.contains(&cause_message) {
            message.push_str(&format!(": {cause_message}"));
        }
        source = cause.source();
    }
    let drift = match error {
        ninja::Error::Status(status) => !status.is_server_error() && status.as_u16() != 429,
        ninja::Error::Parse(_) => true,
        ninja::Error::Transport(_) => false,
    };
    if drift {
        Failure::Drift(message)
    } else {
        Failure::NotRun(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leagues(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    #[test]
    fn challenge_league_is_the_first_temporary_softcore_trade_league() {
        let list = leagues(&[
            "Hardcore Mercenaries",
            "Mercenaries",
            "Standard",
            "Hardcore",
        ]);
        assert_eq!(challenge_league(&list), Some("Mercenaries"));
        let list = leagues(&["SSF Mercenaries", "Ruthless Mercenaries", "Mercenaries"]);
        assert_eq!(challenge_league(&list), Some("Mercenaries"));
    }

    #[test]
    fn no_challenge_league_between_leagues() {
        assert_eq!(challenge_league(&leagues(&["Standard", "Hardcore"])), None);
    }
}
