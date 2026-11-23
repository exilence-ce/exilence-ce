Exilence CE NG
[![Discord Shield](https://discord.com/api/guilds/1157271247214878790/widget.png?style=shield)](https://discord.gg/2T3WXBgjaM)
[![Become a Ko-fi](https://shields.io/badge/ko--fi-Buy_me_a_coffee-ff5f5f?logo=ko-fi&style=for-the-badge)](https://ko-fi.com/ExilenceCE)
===

Desktop client of Exilence CE: tracks the net worth of your Path of Exile stash, alone or in a Group. Tauri 2 core with a Leptos UI.

## Contents

- [Installing](#installing)
- [Changelog](CHANGELOG.md)
- [Linux troubleshooting](#linux-troubleshooting)
- [Contact us](#contact-us)
- [Supporting us](#supporting-us)
- [Acknowledgements](#acknowledgements)
- [License](#license)
- [Releasing](#releasing)
- [poe.ninja drift check](#poeninja-drift-check)

## Installing

Download the installer for your system from the [releases](../../releases) (betas are marked as prereleases): the Windows `…_x64-setup.exe`, the macOS `…_universal.dmg` (Apple silicon and Intel), or on Linux the `…_amd64.AppImage`, the `…_amd64.deb` (Debian, Ubuntu) or the `…_x86_64.rpm` (Fedora, openSUSE; stable releases only). The app updates itself; choose **Latest Stable** or **Beta** under **Settings → Release channel**. A `.deb` or `.rpm` install asks for your password to install an update, and an `.rpm` install always follows Latest Stable: RPM versions cannot name a beta.

### macOS: opening the app past Gatekeeper

The macOS app carries no Apple certificate (it is ad-hoc signed), so Gatekeeper blocks its first start with "Apple could not verify “Exilence CE NG” is free of malware":

1. Drag **Exilence CE NG** from the `.dmg` into **Applications** and open it once; close the warning with **Done**.
2. Open **System Settings → Privacy & Security**, scroll to **Security**, click **Open Anyway** next to the message about Exilence CE NG and confirm with your password.

Or, in Terminal, remove the download's quarantine flag before the first start:

```sh
xattr -dr com.apple.quarantine "/Applications/Exilence CE NG.app"
```

You do this once; updates keep working without it.

### Linux: glibc 2.39 or newer

The AppImage, `.deb` and `.rpm` run on Linux x64 with glibc 2.39 or newer: Ubuntu 24.04, Fedora 40, Debian 13 and later. `ldd --version` shows yours. Make the AppImage executable (`chmod +x Exilence-CE-NG_*_amd64.AppImage`) and start it. If it does not start, install FUSE 2 (`sudo apt install libfuse2t64` on Ubuntu 24.04).

## Linux troubleshooting

### Blank or white window

Exilence CE NG starts with `WEBKIT_DISABLE_DMABUF_RENDERER=1`, which fixes most blank windows on NVIDIA and Mesa drivers; a value you set yourself is kept. If the window is still blank, start the app with compositing off:

```sh
WEBKIT_DISABLE_COMPOSITING_MODE=1 ./Exilence-CE-NG.AppImage
```

This has the same effect as turning off **Settings → Advanced → Hardware acceleration**. To keep it off without the variable, store the setting while the app is closed:

```sh
sqlite3 ~/.local/share/com.exilence.exilence-ce-ng/exilence-ce-ng.db \
  "INSERT OR REPLACE INTO settings (key, value) VALUES ('hardware_acceleration', 'false')"
```

### Net worth overlay on Wayland and tiling window managers

Exilence CE NG starts with `GDK_BACKEND=x11`, so on Wayland it runs through XWayland, where the Net worth overlay can stay on top of the game; a value you set yourself is kept (`GDK_BACKEND=wayland` runs it natively, without an always-on-top overlay). The overlay's window title is always `Exilence CE NG Overlay`. Tiling window managers need rules matched on it:

Hyprland (0.53 or newer, `hyprland.conf`; adjust `move` to where the overlay should sit):

```ini
windowrule = match:title ^(Exilence CE NG Overlay)$, float on, pin on, no_focus on, move 20 80
```

Sway (`~/.config/sway/config`) and i3 (`~/.config/i3/config`):

```
for_window [title="^Exilence CE NG Overlay$"] floating enable, sticky enable
```

### Net worth overlay hidden behind the game on KDE

On KDE Plasma, run Path of Exile in **Windowed Fullscreen** (borderless); in exclusive fullscreen the overlay cannot stay on top of the game.

## Contact us

Communicate with us at our Discord!
[![Discord Server](https://discord.com/api/guilds/1157271247214878790/widget.png?style=banner2)](https://discord.gg/2T3WXBgjaM)

Report bugs at https://github.com/exilence-ce/exilence-ce/issues

## Supporting us

If you feel like chiming in with something, here's our Ko-fi: https://ko-fi.com/ExilenceCE

## Acknowledgements

- https://poe.ninja for providing a great API, which lets us calculate net worth of players

## License

This work is licensed under the GNU Affero General Public License v3.0. See [LICENSE](LICENSE) for the full text.

## Releasing

Maintainers only; the [release workflow](.github/workflows/release.yml) does the rest (ADR 0003 in `exilence-ce/migration`).

1. Set the version in the root `Cargo.toml` (`X.Y.Z` or `X.Y.Z-beta.N`) and add its `## X.Y.Z…` section to `CHANGELOG.md`; it becomes the release notes and the What's New entry.
2. Merge, then push the tag `vX.Y.Z…` on that commit. The workflow runs the tests on Windows and macOS, then builds the Windows installer, the macOS `.dmg` and the Linux AppImage into a draft release (a prerelease for betas).
3. Install the draft on each system and run the [release checklist](docs/release-checklist.md) on it, then publish it; when the `wire` crate changed, the production backend must run it first (the checklist's release steps). Publishing rewrites `latest.json` and `beta.json` on the `updater` release; never edit or delete that release by hand.

A bad release is fixed by a new one. To stop it spreading first, delete it, then run the workflow by hand (**Actions → Release → Run workflow**): the update files then name the newest remaining release. Installs that already have it keep it until a higher version is released.

## poe.ninja drift check

Every Monday the [drift check](.github/workflows/ninja-drift.yml) runs `cargo run -p ninja-drift`: it fetches poe.ninja's price leagues, then every price request of the current challenge league (the first price league that is not Standard, Hardcore, Ruthless or SSF; none between leagues) and Standard with the app's poe.ninja client, and prices the fixture stash in `fixtures/pricing/`. A body that does not parse, or a fixture stash worth 0, fails the run and opens an issue labelled `ninja-drift` (or comments on the open one); poe.ninja being unreachable fails the run without an issue. Run it by hand under **Actions → poe.ninja drift check → Run workflow**; ticking `force_parse_failure` checks a local poe.ninja that does not parse, to test the issue it opens.
