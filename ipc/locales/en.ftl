# English locale resources (D10), read by the core and the UI. Every message
# here is a `MessageKey` in `ipc/src/message.rs` and the other way round.

app-title = Exilence CE NG

notifications-unread = { $count ->
    [0] No unread notifications
    [one] One unread notification
   *[other] { $count } unread notifications
}

status-snapshot-progress = Fetching stash tab { $tab } / { $count }
status-refreshing-stash-tabs = Refreshing stash tabs
status-saving-snapshot = Saving snapshot
snapshot-allowance = GGG allows { $max } requests per { $period } seconds; large snapshots can take a while.
snapshot-allowance-unknown = GGG limits how many requests { app-title } may send; large snapshots can take a while.
snapshot-take = Take snapshot
snapshot-cancel = Cancel snapshot
rate-limit-exceeded = Rate limit exceeded
rate-limit-wait = Rate limit exceeded, please wait: { $seconds } seconds

ui-crashed = Something went wrong. Export diagnostics in Settings and report the issue.

notifications = Notifications
notifications-empty = No notifications

time-just-now = just now
time-minutes-ago = { $count ->
    [one] one minute ago
   *[other] { $count } minutes ago
}
time-hours-ago = { $count ->
    [one] one hour ago
   *[other] { $count } hours ago
}
time-days-ago = { $count ->
    [one] one day ago
   *[other] { $count } days ago
}

login-platform = Platform
platform-pc = PC
platform-xbox = Xbox
platform-playstation = PlayStation
login-authorize = Log in with pathofexile.com
login-copy-link-hint = If the button doesn't work, copy the link and paste it into your browser.
login-copy-link = Copy link
login-signing-in = Signing in…

net-worth = Net worth
net-worth-total = Net worth
net-worth-change-tooltip = Change in value between the two latest snapshots
net-worth-snapshot-count = Snapshot count
net-worth-time-since-tooltip = Time since last snapshot
net-worth-income = Income
net-worth-income-based-on = Based on last hour
net-worth-income-tooltip = Income is calculated based on snapshot values over the last hour
net-worth-hour-suffix = / h
net-worth-income-reset = Reset
overlay-open = Open net worth overlay
overlay-close = Close
currency-chaos = c
currency-exalted = ex
currency-divine = div
currency-switch-rates = 1 ex = { $exalted } c / 1 divine = { $divine } c
currency-switch-hint = Click to switch currency
chart-net-worth = Net worth history chart
chart-tabs = Tab breakdown history chart
chart-time-span-day = 1 day
chart-time-span-week = 1 week
chart-time-span-month = 1 month
chart-time-span-all = All time
chart-reset-zoom = Reset zoom
snapshots-remove = Remove snapshots
snapshots-remove-title = Select snapshots to remove
snapshots-remove-entry = Snapshot { $date }, value: { $value }c
snapshots-items-missing = No items exist on snapshot
select-all = Select all
notification-snapshots-remove-failed = Failed removing snapshots
prices-fetched-from = Prices fetched from
prices-fetched-interval = Updated hourly
prices-unavailable = No prices retrieved
prices-unavailable-title = No prices fetched for `{ $league }`, you can change pricing league in your profile

item-table-icon = Icon
item-table-name = Name
item-table-ilvl-tier = Ilvl / Tier
item-table-tabs = Tabs
item-table-corrupted = Corrupted
item-table-links = Links
item-table-quality = Quality
item-table-level = Level
item-table-quantity = Quantity
item-table-trend = Price last 7 days
item-table-price = Price (c)
item-table-total = Total value (c)
item-table-cumulative = Cumulative (c)
item-table-yes = Yes
item-table-no = No
item-table-not-available = N/A
item-table-icon-alt = Icon fetched from pathofexile.com
item-table-open-on-ninja = View on poe.ninja
item-table-empty = No items found for the selected profile, take a snapshot to begin (top right)
item-table-edit-columns = Edit columns
item-table-visible-columns = Visible columns
item-table-rows-per-page = Rows per page:
item-table-page = { $from }–{ $to } of { $count }
item-table-first-page = First page
item-table-previous-page = Previous page
item-table-next-page = Next page
item-table-last-page = Last page
item-table-title = Items recorded in the latest snapshot
item-table-latest = Only show the latest snapshot
item-table-comparison = Difference for the two latest snapshots
item-table-search = Search items ...
item-table-clear-search = Clear search field
item-table-show-tab-filter = Create filter
item-table-hide-tab-filter = Disable filter
item-table-select-tabs = Select stash tabs
item-table-filter-total = Filter total:
item-table-export = Export data
item-table-column-preset = Select preset
item-table-preset-custom = Custom
item-table-preset-breachstones = Breachstones
item-table-preset-catalysts = Catalysts
item-table-preset-delirium-orbs = Delirium Orbs
item-table-preset-divination-cards = Divination Cards
item-table-preset-essences = Essences
item-table-preset-fossils = Fossils
item-table-preset-incubators = Incubators
item-table-preset-prophecies = Prophecies
item-table-preset-scarabs = Scarabs
item-table-preset-sets = Sets

notification-login-succeeded = Successfully logged in
notification-login-failed = Failed to login
notification-signed-out = Signed out
notification-sign-out-failed = Failed to sign out
notification-session-start-failed = Initialize session failed
notification-snapshot-succeeded = Snapshot created successfully
notification-snapshot-failed = Failed creating snapshot

error-rate-limited = Rate limited. Try again in { $seconds } s.
error-busy = Wait until the running Snapshot has finished.
error-not-ready = A snapshot can be taken once the session, stash tabs and prices have loaded.
error-no-stash-tabs-selected = No stash tabs are selected for the profile
error-offline = Not connected to server
error-session-expired = Session expired, please login again
error-superseded = Your account connected from another device. Restart to reconnect.
error-internal = Something went wrong (reference { $correlation_id }).
error-backend = The server refused the request.
error-ggg-refresh-failed = Path of Exile refused the login. Please login again.
error-group-name-taken = Group already exists
error-group-not-found = Group not found
error-wrong-password = Wrong password
error-not-in-group = You are no longer in the Group.
error-ggg-auth-failed = Path of Exile login failed.
error-login-rejected = The login link does not belong to this login attempt. Please try again.
error-login-refused = The authorization was refused on pathofexile.com.
error-browser-not-opened = The browser did not open. Copy the link and paste it into your browser.
error-clipboard = The link could not be copied.
error-ggg-token-expired = Your Path of Exile login expired and could not be renewed yet.

title-bar-version = v{ $version }
title-bar-kofi = Support us on Ko-fi
title-bar-support = Support
title-bar-minimize = Minimize
title-bar-maximize = Maximize
title-bar-restore = Restore
title-bar-close = Close
title-bar-update-ready = Update available, click here to update now!

support-discord = Discord
support-whats-new = What's new
support-retake-tour = Re-take tour
support-feature-request = Feature request
support-bug-report = Bug report
support-us = Support us
support-credits = Credits

whats-new-title = Releases
info-dialog-loading = Loading…
info-dialog-unavailable = This could not be loaded from GitHub. Check your connection and try again.
info-dialog-close = Close

tour-back = Back
tour-next = Next
tour-lets-begin = Let's begin!
tour-initial-title = Welcome to Exilence CE NG!
tour-initial-body = This tour will guide you through the main functions of the app to get you started quickly.
tour-net-worth-title = Net worth view
tour-net-worth-body = In this view you can track all your gains and losses and how much your stash is worth.
tour-settings-title = Settings view
tour-settings-body = In this view you can manage your settings.
tour-profile-title = Profile section
tour-profile-body = In this area you can manage your profiles, and create multiple ones for different setups for quick access.
tour-profile-body2 = Profiles are the core of the app where you select your stash tabs to track.
tour-snapshot-title = Snapshot section
tour-snapshot-body = Here you take snapshots to record your current net worth, and you can also enable automatic snapshotting in the settings.
tour-snapshot-body2 = A snapshot represents a recording of your current stash tabs (controlled by the profile).
tour-overlay-title = Overlay section
tour-overlay-body = Here you can toggle the net worth overlay, for a slim overview while you map!
tour-group-title = Group section
tour-group-body = Here you can group up with your friends to see your combined net worth status.
tour-notifications-title = Notification list
tour-notifications-body = Everything that happens in the app will be displayed here.
tour-currency-switch-title = Switch currency
tour-currency-switch-body = Clicking this button switches the values between exalted, divine and chaos.
tour-reset-income-title = Reset income
tour-reset-income-body = Clicking this button resets your income.
tour-support-panel-title = Support panel
tour-support-panel-body = Here you can see what's new in the latest release, report a bug, submit a feature request, re-run this tour or get our Discord server invitation.

nav-open = Open navigation
nav-close = Close navigation
nav-net-worth = Net worth
nav-settings = Settings

account-menu = Account
account-sign-out = Sign out

settings-exit-action = When clicking X (close window)
settings-exit-action-minimize-to-tray = Minimize { app-title } to the system tray
settings-exit-action-exit = Exit { app-title } completely

settings-release-channel = Current release channel:
settings-release-channel-latest = Latest Stable
settings-release-channel-beta = Beta
settings-release-channel-note = Applies after { app-title } restarts. This setting is for this computer only.

profile-switcher = Profile
profile-edit = Edit profile
profile-create = Create profile
profile-remove = Remove profile
profile-dialog-create-title = Create new profile
profile-dialog-edit-title = Edit profile
profile-name = Profile name
profile-name-placeholder = Enter a profile name
profile-generate-name = Generate a random name
profile-price-league = Select pricing league
profile-league = Select league
profile-stash-tabs = Select stash tabs
profile-stash-tabs-warning = Selecting many tabs (especially Map / Unique tab) can increase the snapshot duration significantly
profile-stash-tabs-placeholder = Add/remove tabs
profile-stash-tabs-none = No options
profile-stash-tabs-clear = Clear
profile-stash-tabs-open = Open
profile-stash-tabs-close = Close
profile-character = Select character
profile-no-character = None
profile-include-equipment = Include equipment
profile-include-inventory = Include inventory
profile-options-loading = Loading leagues, stash tabs and characters…
profile-dialog-create = Create profile
profile-dialog-save = Save profile
cancel = Cancel
profile-remove-title = Remove profile
profile-remove-body = Are you sure you want to remove your profile?
profile-remove-confirm = Remove

error-profile-name-required = Required
error-no-characters-in-league = You have no characters in the selected league
error-price-leagues = The pricing leagues could not be loaded from poe.ninja.

notification-profile-created = Successfully created profile
notification-profile-create-failed = Failed to create profile
notification-profile-updated = Successfully updated profile
notification-profile-update-failed = Failed to update profile
notification-profile-changed = Successfully changed profile
notification-profile-change-failed = Failed to change profile
notification-profile-removed = Remove profile succeeded
notification-profile-remove-failed = Remove profile failed
notification-profile-options-failed = Failed to load stash tabs and characters
settings-tab-general = General
settings-tab-ui = Interface
settings-tab-prices = Prices
settings-tab-advanced = Advanced

settings-section-exit-action = Application on exit action
settings-section-snapshot = Snapshot settings
settings-section-pricing = Pricing settings
settings-section-interface = General
settings-section-prices = Prices
settings-section-release-channel = Release Channel Settings
settings-section-hardware-acceleration = Hardware Acceleration
settings-requires-snapshot = * = requires a new snapshot to take effect

settings-auto-snapshot-label = Activate auto snapshotting
settings-auto-snapshot = Auto snapshotting
settings-auto-snapshot-help = Handles snapshotting for you at set intervals
settings-auto-snapshot-minutes = Set auto snapshot interval
settings-auto-snapshot-minutes-help = Sets the interval for auto snapshotting
settings-unit-minutes = min
settings-low-confidence-pricing-label = Activate low confidence pricing
settings-low-confidence-pricing = Low confidence
settings-low-confidence-pricing-help = Includes prices with less than 10 listings
settings-price-threshold = Price threshold
settings-price-threshold-help = Sets the cutoff for including items
settings-total-price-threshold = Total price threshold
settings-total-price-threshold-help = Sets the cutoff for including total stack of items
settings-exempt-lifeforce-and-markers-label = Exclude from price threshold
settings-exempt-lifeforce-and-markers = Harvest lifeforce & Rogue's Markers
settings-exempt-lifeforce-and-markers-help = Includes Crystallised Lifeforce and Rogue's Markers even below the price threshold

settings-ui-scale = Interface scale (%)
settings-ui-scale-help = Sets the UI scale
settings-ui-scale-reset = Reset to default

settings-hardware-acceleration-label = Enable Hardware Acceleration
settings-hardware-acceleration = Hardware Acceleration
settings-hardware-acceleration-note = Applies after { app-title } restarts.

settings-reset-data = Reset Data
settings-reset-data-description = Deletes all { app-title } data on this computer (Snapshots, settings, window positions), signs you out and restarts { app-title }. Logs and legacy Exilence CE data stay.
settings-reset-data-question = Are you sure you want to reset your data?
settings-reset-data-confirm = Confirm
settings-reset-data-cancel = Cancel

settings-debug-mode-label = Activate debug mode
settings-debug-mode = Debug mode
settings-debug-mode-note = Logs every request and step in detail, for diagnosing a problem. Applies at once and stays on until you turn it off.
settings-diagnostics = Diagnostics
settings-diagnostics-note = The Diagnostics bundle holds the logs, versions, system details and settings, never tokens or passwords. Nothing is uploaded: attach it to a bug report yourself.
export-diagnostics = Export diagnostics
open-log-folder = Open log folder
debug-marker = DEBUG
debug-marker-title = Debug mode is on
notification-last-session-crashed = The last session ended unexpectedly
notification-last-session-crashed-description = Export diagnostics in Settings → Advanced and attach the bundle to a bug report.
notification-diagnostics-exported = Diagnostics bundle saved to { $path }
notification-diagnostics-export-failed = Failed to export diagnostics
notification-csv-export-failed = Failed to export the item table
notification-log-folder-failed = Failed to open the log folder

error-setting-out-of-range = This value is out of range.

custom-prices-price-league = Price league
custom-prices-filter = Search items ...
custom-prices-clear-filter = Clear search field
custom-prices-waiting = Waiting for response from price source...
custom-prices-empty = No prices found
custom-prices-icon = Icon
custom-prices-name = Name
custom-prices-ilvl-tier = Ilvl / Tier
custom-prices-variant = Variant
custom-prices-corrupted = Corrupted
custom-prices-links = Links
custom-prices-quality = Quality
custom-prices-level = Level
custom-prices-price = Price
custom-prices-custom-price = Custom price
custom-prices-no-data = No data
custom-prices-not-set = Not set
custom-prices-set = Set custom price
custom-prices-remove = Remove custom price
custom-price-dialog-title = Set custom price
custom-price-dialog-placeholder = Custom price
custom-price-dialog-close = Close
custom-price-dialog-save = Save
error-custom-price-required = Required
error-custom-price-invalid = Enter a price of 0 or more.

status-importing-legacy = Importing Exilence CE history
legacy-import-title = Import from Exilence CE
legacy-import-intro = Exilence CE data was found on this computer. Choose the Exilence CE accounts whose profiles, snapshots, custom prices and settings you want to bring over.
legacy-import-note = Everything ticked goes into your account { $account }. Settings and custom prices already set in { app-title } are kept. Imported snapshots stay on this computer.
legacy-import-snapshot-count = { $count ->
    [one] One snapshot
   *[other] { $count } snapshots
}
legacy-import-date-range = { $from } – { $to }
legacy-import-unnamed-profile = Unnamed profile
legacy-import-no-profiles = No profiles
legacy-import-confirm = Import
legacy-import-skip = Skip
settings-legacy-import-note = Brings profiles, snapshots, custom prices and settings over from Exilence CE on this computer. Importing again adds nothing twice.
notification-legacy-import-succeeded = Imported { $profiles ->
    [one] one profile
   *[other] { $profiles } profiles
} and { $snapshots ->
    [one] one snapshot
   *[other] { $snapshots } snapshots
}
notification-legacy-import-failed = Import from Exilence CE failed
legacy-import-no-data = No Exilence CE data found on this computer
error-legacy-running = Exilence CE is running. Quit Exilence CE and try again.
error-legacy-unreadable = The Exilence CE data could not be read.

tray-show = Show { app-title }
tray-check-for-updates = Check for Updates…
tray-donate = Donate
tray-discord-help = Discord / Help
tray-quit = Quit
tray-update-none = No updates available
tray-update-available = Update { $version } available
tray-update-downloading = Downloading update { $version }: { $percent } %
tray-update-ready = Update { $version } ready to install

group-open = Groups
group-close = Close groups
group-create = Create group
group-join = Join group
group-leave = Leave group
group-players = Players
group-create-title = Create new group
group-join-title = Join existing group
group-name = Group name
group-name-placeholder = Enter a group name
group-generate-name = Generate a random name
group-password = Password
group-password-placeholder = Enter password here
group-password-helper = Must be at least 6 characters in length
group-leave-title = Leave group
group-leave-body = Are you sure you want to leave your current group?
error-group-name-required = Required
error-group-password-too-short = Must be at least 6 characters in length
notification-group-joined = Successfully joined group
notification-group-join-failed = Failed to join group
notification-group-left = Successfully left group
notification-group-leave-failed = Failed to leave group
