# Release checklist

What a maintainer checks by hand before publishing a release: everything the automated tests cannot see (windows, tray, overlay, deep link, installers, updater, look). Specs in `exilence-ce/migration`: [parity verification](https://github.com/exilence-ce/migration/blob/main/docs/spec/parity-verification.md#manual-release-checklist) (the Manual column of its per-feature table, row by row below), the [parity checklist](https://github.com/exilence-ce/migration/blob/main/docs/spec/parity-checklist.md) (`F<n>`/`D<n>`), [backend deployment](https://github.com/exilence-ce/migration/blob/main/docs/spec/backend-deployment.md#build) and ADR 0003.

Every check runs on the **draft release**, installed from its assets, **signed in against the production backend**. There is no staging backend, and GGG's fixed OAuth redirect keeps a local backend from completing a login. Tick a copy of the tables for each release; this file stays unticked.

## Which pass

| Release | Windows, macOS, Linux | Linux matrix |
|---|---|---|
| `2.0.0-beta.1`, `2.0.0` | **Full pass**: the [smoke pass](#smoke-pass), every row of [per feature](#per-feature) and [D9, D10](#full-pass-only) | yes |
| any other | the [smoke pass](#smoke-pass), plus the rows of every feature the release changes (its `CHANGELOG.md` section and the diff since the previous release name them) | when [its triggers](#linux-matrix) apply |

Columns **Win**, **mac** and **Linux** are the three OSes; in a full pass every ☐ is ticked, in other releases those of the rows that run. `—` means the row does not apply to that OS. The Linux column runs on one Linux desktop; the [Linux matrix](#linux-matrix) extends it to F14's environments.

## Release steps

1. **Version and changelog.** Set the version in the root `Cargo.toml` (`X.Y.Z` or `X.Y.Z-beta.N`) and add its `## X.Y.Z…` section to `CHANGELOG.md`: it becomes the release notes and the What's New entry. Merge.
2. **Tag.** Push `vX.Y.Z…` on the merged commit. The [release workflow](../.github/workflows/release.yml) checks that the tag names the workspace version and that `CHANGELOG.md` has its section, runs the tests on Windows and macOS, and builds the Windows installer, the macOS `.dmg` and the Linux AppImage.
3. **Draft.** Wait for the draft release `vX.Y.Z…` (a prerelease for betas) with `Exilence-CE-NG_<version>_x64-setup.exe`, `_universal.dmg`, `_universal.app.tar.gz`, `_amd64.AppImage` and a `.sig` beside each update artifact. Its notes are the version's `CHANGELOG.md` section.
4. **Backend `wire`.** The backend pins `wire` by a release tag of this repo (`wire = { git = "…", tag = "vX.Y.Z…" }` in `exilence-ce-ng-backend`'s `Cargo.toml`), and a client release whose `wire` crate changed is published only after the production backend runs it (the `ItemDefinition` hash check). With `vPIN` the tag the backend release in the host's `compose.yaml` pins:
   - `git diff --quiet vPIN vX.Y.Z… -- wire/` succeeds → nothing to do.
   - Otherwise set the backend's `wire` tag to `vX.Y.Z…` (`cargo update -p wire`), release and deploy the backend (backend `DEPLOY.md`), then continue. A pushed release tag never moves.
5. **Check.** Install the draft on Windows, macOS and Linux (for the update row, over the previous release) and run [the pass this release needs](#which-pass) against the production backend. A failed row stops the release: fix, bump, tag again.
6. **Publish.** Re-check step 4 if the backend changed meanwhile, then publish the draft. Publishing runs the workflow's **Rewrite the update files** job.
7. **Verify the update files.** After that job succeeds, `gh release download updater -p latest.json -p beta.json -O -` shows: `beta.json` names the higher of the newest stable and the newest beta release, `latest.json` the newest stable (no `latest.json` before the first stable), each with `windows-x86_64`, `darwin-aarch64`, `darwin-x86_64` and `linux-x86_64` URLs into that release and their signatures. Never edit or delete the `updater` release by hand. A bad release is fixed by a new one ([README](../README.md#releasing)).

## Smoke pass

Every release, on each OS, with the installed draft.

| # | Check | Win | mac | Linux |
|---|---|---|---|---|
| S1 | **Sign in**: **Log in with pathofexile.com**, approve in the browser; the deep link returns to the running app on Net worth. | ☐ | ☐ | ☐ |
| S2 | **Snapshot**: **Take snapshot** finishes; widgets, charts and item table show it. | ☐ | ☐ | ☐ |
| S3 | **Group join with two accounts**: account A creates a Group, account B (another machine or OS) joins it with the password; both see each other selected with their latest Snapshot, a new Snapshot of either appears at the other without action, Leave removes the player from the other's view. | ☐ | ☐ | ☐ |
| S4 | **Overlay**: the toolbar button opens the Net worth overlay on top of the game; it shows net worth and income and updates after a Snapshot. | ☐ | ☐ | ☐ |
| S5 | **Update from the previous release**: install the previous release, set its channel to the one this release is on, publish this release; within 5 minutes the title bar shows "Update available, click here to update now!", clicking it restarts into this version with data and sign-in kept. Runs after step 6. | ☐ | ☐ | ☐ |

## Per feature

One row per item of the Manual column of the per-feature table.

### F1. Login and session

| # | Check | Win | mac | Linux |
|---|---|---|---|---|
| F1.1 | **Platform choice**: PC / Xbox / PlayStation offered on the login screen; the choice is remembered after a restart. | ☐ | ☐ | ☐ |
| F1.2 | **Browser opens**: **Log in with pathofexile.com** opens GGG's consent page in the system browser, asking for stashes, profile and characters. | ☐ | ☐ | ☐ |
| F1.3 | **Copy link**: **Copy link** puts the authorization URL on the clipboard; pasted into a browser it opens the same consent page and completes the login. | ☐ | ☐ | ☐ |
| F1.4 | **Deep link into the running app on each OS**: approving brings the already running app to Net worth through `exilence-ce-ng://`, without a second login or a second app window. | ☐ | ☐ | ☐ |
| F1.5 | **Error toast**: refusing on the consent page shows an error toast ("The authorization was refused on pathofexile.com.") and returns to the login screen. | ☐ | ☐ | ☐ |

### F2. Profiles

| # | Check | Win | mac | Linux |
|---|---|---|---|---|
| F2.1 | **Dialog fields**: name (required, random-name button), price league (poe.ninja's leagues), league, stash tabs (multi-select, folders replaced by their child tabs), character, include equipment, include inventory. | ☐ | ☐ | ☐ |
| F2.2 | **Disabled states**: include options disabled until a character is chosen; changing the league resets character and include options; submit disabled while the form is invalid or the league has no characters; the Profile switcher disabled while a Snapshot runs; Remove disabled with one Profile. | ☐ | ☐ | ☐ |
| F2.3 | **5-tab warning**: selecting 5 or more stash tabs shows the warning. | ☐ | ☐ | ☐ |
| F2.4 | **Click outside**: clicking outside the dialog does not close it. | ☐ | ☐ | ☐ |

### F3. Taking Snapshots

| # | Check | Win | mac | Linux |
|---|---|---|---|---|
| F3.1 | **Status text**: a running Snapshot shows "Fetching stash tab i / N" with the info hint naming GGG's request allowance; **Cancel** clears it. | ☐ | ☐ | ☐ |
| F3.2 | **Cancel disabled while saving**: once the Snapshot is being saved, **Cancel** is disabled. | ☐ | ☐ | ☐ |

### F4. Auto-snapshot

| # | Check | Win | mac | Linux |
|---|---|---|---|---|
| F4.1 | **Interval field disabled while off**: Settings → General: the interval field (5–1000, default 20) is disabled while auto snapshotting is off and enabled when on. | ☐ | ☐ | ☐ |

### F5. Pricing

| # | Check | Win | mac | Linux |
|---|---|---|---|---|
| F5.1 | **Fetched-ago header**: the item table's section header ("Items recorded in the latest snapshot") shows "Prices fetched from https://poe.ninja … ago", counts up, and the link opens poe.ninja in the browser without folding the section. | ☐ | ☐ | ☐ |
| F5.2 | **No-prices warning**: while poe.ninja has no prices for the Profile's pricing league (e.g. right after a league start), the toolbar shows "No prices retrieved" with a title naming the league. Note "not reproducible" if no such league exists at release time. | ☐ | ☐ | ☐ |

### F6. Custom prices

| # | Check | Win | mac | Linux |
|---|---|---|---|---|
| F6.1 | **Table**: Settings → Prices shows the chosen league's price table, 25 per page, with edit and delete actions. | ☐ | ☐ | ☐ |
| F6.2 | **Filter**: the name filter narrows the table. | ☐ | ☐ | ☐ |
| F6.3 | **Waiting text**: "Waiting for response from price source..." shows until prices are available (e.g. right after start or on choosing another league). | ☐ | ☐ | ☐ |
| F6.4 | **Dialog**: edit opens a dialog with a required number prefilled with the price rounded to 2 decimals; saving stores it (kept after a restart), delete removes it. | ☐ | ☐ | ☐ |

### F7. GGG rate limits

| # | Check | Win | mac | Linux |
|---|---|---|---|---|
| F7.1 | **Countdown display**: after GGG answers 429 (e.g. another tool spending the account's allowance), the toolbar shows "Rate limit exceeded, please wait: N seconds" counting down, Snapshot disabled until it ends. | ☐ | ☐ | ☐ |

### F8. Net worth widgets

| # | Check | Win | mac | Linux |
|---|---|---|---|---|
| F8.1 | **Widgets**: Net worth with sparkline and change, Income "/ h" "Based on last hour" with **Reset** (hidden in a Group), Snapshot count and time since the last Snapshot ticking every second. | ☐ | ☐ | ☐ |
| F8.2 | **Colours**: the change is green when positive, red when negative. | ☐ | ☐ | ☐ |
| F8.3 | **Tooltip**: the change shows "Change in value between the two latest snapshots". | ☐ | ☐ | ☐ |

### F9. Currency display

| # | Check | Win | mac | Linux |
|---|---|---|---|---|
| F9.1 | **Switch cycle**: the switch cycles `ex` → `div` → `c` and is remembered after a restart. | ☐ | ☐ | ☐ |
| F9.2 | **Tooltip**: `1 ex = X.X c / 1 divine = X.X c` with the pricing league's rates, custom prices included. | ☐ | ☐ | ☐ |
| F9.3 | **Instant update**: switching updates the widgets, the item-table subtotal and the overlay at once. | ☐ | ☐ | ☐ |

### F10. Charts

| # | Check | Win | mac | Linux |
|---|---|---|---|---|
| F10.1 | **Look (D24)**: net worth history filled with a vertical gradient of the primary colour, legacy palette, straight lines between Snapshots, nothing animates on load; sparklines (Net worth, "Price last 7 days" in the item table and Settings → Prices) smoothed, broken on days without data, filled to the zero line (under a rise, over a fall). | ☐ | ☐ | ☐ |
| F10.2 | **Tooltip**: hovering shows the nearest point's local date and time, series name and value in chaos with exactly 2 decimals in the system number format. | ☐ | ☐ | ☐ |
| F10.3 | **Zoom**: dragging across the time axis zooms in; **Reset zoom** restores; changing the time span resets the zoom. | ☐ | ☐ | ☐ |
| F10.4 | **Legend toggle**: clicking a Tab breakdown legend entry hides and shows that tab's line. | ☐ | ☐ | ☐ |
| F10.5 | **Sections remembered**: collapsing or opening the chart and item sections, and the time span, survive a restart (first start: charts collapsed, items open). | ☐ | ☐ | ☐ |

### F11. Item table

| # | Check | Win | mac | Linux |
|---|---|---|---|---|
| F11.1 | **Columns**: Latest shows Icon, Name, Ilvl/Tier, Tabs, Corrupted, Links, Quality, Level, Quantity, Price last 7 days, Price, Total value, Cumulative; Comparison hides Tabs and Cumulative and shows `+ ` and coloured diffs. | ☐ | ☐ | ☐ |
| F11.2 | **Presets**: preset picker and per-column checkboxes; the last visible column cannot be hidden; toggling a column updates the active preset; both remembered after a restart. | ☐ | ☐ | ☐ |
| F11.3 | **Widths and page size remembered**: changed column widths, sort and page size (5/10/25/50) survive a restart. | ☐ | ☐ | ☐ |
| F11.4 | **Rarity colours**: names are coloured by rarity (normal, magic, rare, unique, gem, currency, divination). | ☐ | ☐ | ☐ |
| F11.5 | **Links**: a name opens its poe.ninja page in the system browser. | ☐ | ☐ | ☐ |

### F12. Snapshot retention and removal

| # | Check | Win | mac | Linux |
|---|---|---|---|---|
| F12.1 | **Dialog**: the toolbar's remove-Snapshots button lists `Snapshot YYYY-MM-DD HH:mm, value: X.XXc`; remove is disabled until something is selected; cancel closes; removed Snapshots leave every view. | ☐ | ☐ | ☐ |
| F12.2 | **Marks**: Snapshots older than the newest 10 (or imported ones) are marked as without item detail. | ☐ | ☐ | ☐ |
| F12.3 | **Select all**: selects every entry. | ☐ | ☐ | ☐ |

### F13. Group

| # | Check | Win | mac | Linux |
|---|---|---|---|---|
| F13.1 | **Two-account smoke**: smoke pass S3 with the two accounts on two OSes. | ☐ | ☐ | ☐ |
| F13.2 | **Drawer**: **Groups** opens the drawer and closes the navigation; Create suggests a random name (dice re-rolls); taken / not-found names show when the name field loses focus; a wrong password shows on the password field; in a Group the drawer shows the name, the player checkboxes and Leave with confirmation. | ☐ | ☐ | ☐ |

### F14. Net worth overlay

| # | Check | Win | mac | Linux |
|---|---|---|---|---|
| F14.1 | **Linux matrix**: every required [Linux matrix](#linux-matrix) environment passes (when the matrix runs). | — | — | ☐ |
| F14.2 | **Window behaviour on each OS**: frameless, always on top of the game, no taskbar or dock entry; shows "Net worth X.XX <currency>" and "Income <value> / h" (Group values in a Group); draggable by its header; X closes it; size (minimum 200×92) and position remembered after reopening and after a restart. | ☐ | ☐ | ☐ |

### F15. Desktop shell

| # | Check | Win | mac | Linux |
|---|---|---|---|---|
| F15.1 | **Title bar**: title, version, update banner slot, ko-fi, support button, minimize, maximize/restore, close; main window size and position remembered (minimum 1150×800). | ☐ | ☐ | ☐ |
| F15.2 | **Tray incl. sublabel (D13)**: tray menu Show, Check for Updates…, Donate, Discord / Help, Quit; left-click shows the window (where the OS supports it). The disabled item below Check for Updates… ("No updates available", "Update X available", "Downloading update…", "Update X ready to install") follows the real update state. | ☐ | ☐ | ☐ |
| F15.3 | **Second instance**: starting the app again focuses the running window and opens no second one. | ☐ | ☐ | ☐ |
| F15.4 | **Close action**: with "Minimize Exilence CE NG to the system tray" close hides to the tray; with "Exit Exilence CE NG completely" close quits. | ☐ | ☐ | ☐ |
| F15.5 | **Linux matrix**: the window renders without user action in every required [Linux matrix](#linux-matrix) environment (when the matrix runs). | — | — | ☐ |

### F16. Updates

| # | Check | Win | mac | Linux |
|---|---|---|---|---|
| F16.1 | **Update from the previous release**: smoke pass S5. | ☐ | ☐ | ☐ |
| F16.2 | **A tampered update file rejected and logged**: on a previous-release install whose OS trusts a local `mitmproxy` CA and uses it as system proxy, rewrite the channel file (`…/releases/download/updater/<channel>.json`) so its signature for this OS is another artifact's `.sig`. No title-bar banner appears, nothing installs, and the log has `update check on … failed`. | ☐ | ☐ | ☐ |
| F16.3 | **Title-bar banner**: once downloaded, "Update available, click here to update now!" shows; clicking cancels a running Snapshot and restarts into the new version. | ☐ | ☐ | ☐ |
| F16.4 | **Installers**: Windows `_x64-setup.exe` installs for the current user into a folder you may choose; the macOS `_universal.dmg` runs on Apple silicon and Intel after the README's Gatekeeper steps; the Linux `_amd64.AppImage` starts on glibc 2.39 (Ubuntu 24.04), and after its first start `Exilence-CE-NG` shows with its icon in the desktop's app menu or launcher and starts from there. Betas are marked prerelease. | ☐ | ☐ | ☐ |
| F16.5 | **README**: the [README](../README.md) install section matches the assets, the Gatekeeper steps work as written, the glibc minimum is stated. | ☐ | ☐ | ☐ |
| F16.6 | **Release notes**: the draft's notes equal the version's `CHANGELOG.md` section, which What's New shows too. | ☐ | ☐ | ☐ |
| F16.7 | **`.deb` and `.rpm`**: on Ubuntu 24.04 `sudo apt install ./…_amd64.deb` and, for a stable release, on Fedora `sudo dnf install ./…_x86_64.rpm` install the app; `Exilence-CE-NG` is in the app menu with its icon, a login link from the browser reaches the running app, and an update from the previous release asks for the password, installs and restarts into the new version. The `.rpm` install shows no release channel; a beta draft has no `.rpm`. | — | — | ☐ |
| F16.8 | **AUR** (stable releases, once the repository variable `AUR_PUBLISH` is `true`; skip while it is off): after publishing, the release workflow's "Update the AUR packages" job succeeded; on Arch `paru -S exilence-ce-bin` (then, removing it, `exilence-ce`) installs the published version; the app menu entry and a login link work; Settings → Advanced has no release channel, the tray's "Check for Updates…" is disabled and no update banner ever shows. | — | — | ☐ |

### F17. Settings

| # | Check | Win | mac | Linux |
|---|---|---|---|---|
| F17.1 | **Tabs**: Net worth, Interface, Custom prices, Advanced. | ☐ | ☐ | ☐ |
| F17.2 | **UI scale**: Interface scale 50–150 % in steps of 10 applies at once, Reset to default restores 100, kept after a restart. | ☐ | ☐ | ☐ |
| F17.3 | **Hardware acceleration (D7, D16)**: Settings → Advanced shows the stored value (D16); turning it off applies after a restart, on Windows (no GPU in WebView2) and Linux; hidden on macOS. | ☐ | ☐ | ☐ |
| F17.4 | **Reset data**: Settings → Advanced → Reset data asks for confirmation, then restarts signed out with default settings; legacy Exilence CE data and the logs stay. On Windows also run the [Reset data check](#reset-data-check-windows). | ☐ | ☐ ([#105](https://github.com/exilence-ce/migration/issues/105)) | ☐ |

#### Reset data check (Windows)

WebView2's browser process can keep the webview data in use after the app exits (exilence-ce/migration#121). Signed in, with the app running, in PowerShell:

```powershell
$local = "$env:LOCALAPPDATA\com.exilence.exilence-ce-ng"
New-Item "$local\EBWebView\before-reset.txt" -Value "old" | Out-Null
Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" |
  Where-Object { $_.CommandLine -like '*com.exilence.exilence-ce-ng*' -and $_.CommandLine -notlike '*--type=*' } |
  Select-Object ProcessId
```

Note the one `ProcessId`. Run Reset data, wait for the login screen, then:

```powershell
Test-Path "$local\EBWebView\before-reset.txt"
Test-Path "$env:APPDATA\com.exilence.exilence-ce-ng\.reset-data"
Select-String "$local\logs\exilence-ce-ng.log" -Pattern 'resetting data|browser process|deleting|cannot delete|os error 32' |
  Select-Object -Last 10 | ForEach-Object Line
```

Passes when both `Test-Path` print `False` and the log ends with `resetting data`, `waiting for webview browser process <ProcessId> to exit`, `webview browser process <ProcessId> exited` and `deleting local data (Reset data)`, with no `still runs`, `cannot delete` or `os error 32`.

### F18. Notifications and status

| # | Check | Win | mac | Linux |
|---|---|---|---|---|
| F18.1 | **Bell**: lists the newest 10 with type icon, title, relative time and description; the badge counts unread (up to 9); opening marks all read. | ☐ | ☐ | ☐ |
| F18.2 | **Spinner**: the status spinner shows the current status message while working (Snapshot, sign-in, import). | ☐ | ☐ | ☐ |

### F19. Onboarding, What's New, Credits, support

| # | Check | Win | mac | Linux |
|---|---|---|---|---|
| F19.1 | **Tour**: runs on first launch (11 steps); Re-take tour in the support panel runs it again (not on the login screen). | ☐ | ☐ | ☐ |
| F19.2 | **What's New**: opens once after the first-run tour and from the support panel, showing this repo's `CHANGELOG.md` as rendered markdown. | ☐ | ☐ | ☐ |
| F19.3 | **Credits**: shows this repo's `CREDITS.md` as rendered markdown. | ☐ | ☐ | ☐ |
| F19.4 | **Support links (D11)**: Discord, Feature request and Bug report (this repo's issue templates), Support us (ko-fi) open in the system browser. | ☐ | ☐ | ☐ |

### F20. Debug mode and diagnostics

| # | Check | Win | mac | Linux |
|---|---|---|---|---|
| F20.1 | **Export dialog**: **Export diagnostics** (Settings and support panel) opens a save dialog; the zip holds logs, version and channel, OS, webview version, settings without secrets, rate-limit state; on Linux also session type, GPU and the `GDK_BACKEND`/`WEBKIT_*` values. No token or password in it. | ☐ | ☐ | ☐ |
| F20.2 | **Open log folder**: opens the log directory in the file manager. | ☐ | ☐ | ☐ |
| F20.3 | **Debug marker**: turning on Debug mode shows `DEBUG` in the toolbar at once; starting with `--debug` or `EXILENCE_LOG` shows it from launch. | ☐ | ☐ | ☐ |
| F20.4 | **Bug template**: the Bug report link opens this repo's bug form asking for the Diagnostics bundle. | ☐ | ☐ | ☐ |

### F21. Legacy import

| # | Check | Win | mac | Linux |
|---|---|---|---|---|
| F21.1 | **One real Windows import before `beta.1`**: once, on the `2.0.0-beta.1` draft, import a real Exilence CE folder; Snapshot count and newest net worth match the legacy app. | ☐ | — | — |

### Full pass only

| # | Check | Win | mac | Linux |
|---|---|---|---|---|
| D9 | **No consent dialog**: no analytics or consent dialog appears on first launch or later. | ☐ | ☐ | ☐ |
| D10 | **Strings in locale resources**: every screen shows English text; no raw message id (`kebab-case-key`) or `{$name}` placeholder anywhere. | ☐ | ☐ | ☐ |

## Linux matrix

F14 (overlay) and the Linux part of F15 (window rendering, tray, close action) in each environment, with the AppImage of the draft and the [README](../README.md#linux-troubleshooting) rules where named.

Runs before `2.0.0-beta.1`, before `2.0.0`, and in any release that changes the Tauri, wry or WebKitGTK version, the overlay window code, or the `GDK_BACKEND`/`WEBKIT_*` defaults. Desktop environments run in VMs; the NVIDIA row runs on the maintainer's NVIDIA machine.

| Environment | Where | Required | Window renders | Tray | Overlay on top of the game |
|---|---|---|---|---|---|
| GNOME Wayland (Ubuntu 24.04 LTS or newer) | VM | yes | ☐ | ☐ | ☐ |
| KDE Plasma Wayland | VM | yes | ☐ | ☐ | ☐ above a borderless or windowed game |
| Cinnamon X11 | VM | yes | ☐ | ☐ | ☐ |
| Hyprland with the README rules | VM | yes | ☐ | ☐ | ☐ floats and stays above |
| NVIDIA proprietary driver | maintainer's NVIDIA machine | yes | ☐ | ☐ | ☐ |
| Cinnamon Wayland | VM | best effort | ☐ | ☐ | ☐ |
| Sway with the README rules | VM | best effort | ☐ | ☐ | ☐ |
| i3 with the README rules | VM | best effort | ☐ | ☐ | ☐ |
