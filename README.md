# Tote

Tote finds a teacher's files and files the ones she sends it with one right-click, with no other routine to learn. Built for one real user first: Lex's mother, a public school teacher on a Windows laptop whose files are saved everywhere.

What it does:

- Files what she sends it: right-click a file in File Explorer and choose **Move to Tote** (under Show more options on Windows 11). Tote puts it in `Documents\School Files` by grade and subject from the name (the Organize rule), else by type and month, answers with a Windows message, and Activity can undo it. Nothing moves on its own (Lex's decision, 2026-10-17); a download keeps where it came from (Messenger, Gmail, Google Drive, DepEd, or the site).
- Indexes Desktop, Documents, Downloads, Pictures, Videos, Music and OneDrive, including the words inside Word, Excel, PowerPoint, PDF and text files, so typing a student's name finds `Book1.xlsx`. Build output inside code projects is skipped.
- Notices downloads a browser saved somewhere else (by the browser's web stamp) and lists them under Recent downloads without moving them, and checks each browser's download settings.
- One Find screen: a big search box, Recent downloads, pinned files, date and type and source filters, copies grouped with the newest marked Latest, a quick preview, and Copy to paste a file into a message.
- Organizing by hand when she wants to: Move to, Rename (extension locked), New folder, Recycle Bin, Pin. Every change is logged in Activity with Undo.
- Clean up: duplicate copies, loose Desktop files and old downloads, each with a suggestion. Nothing moves until she clicks.
- To do: tasks with an optional due date, weekly, monthly or quarterly repeats, and attached files that open in one click. One Windows message on the morning a task is due. The list lives in its own file (`tasks.sqlite`) with a daily copy, so a damaged database never empties it.
- A Templates shelf that makes a dated copy of a master file instead of editing the original.
- Settings: start at login, notifications, school-year dates, templates (native file picker), and the folders Tote looks in (add with a native folder picker, remove all but Documents and Downloads; changes apply after a restart).
- A five-step walkthrough with arrows on first run, replayable from Settings.
- No network access except one small check: at most once a day Tote reads https://www.codewithlex.site/tote/latest.json to see if a newer version exists (Settings > Check for new versions turns it off). Nothing about her files is sent.
- New versions install with one click from a banner; an update is installed only if its signature matches the public key built into Tote.

The binding design is `docs/superpowers/specs/2026-10-12-teachers-desktop-manager-design.md`. The surface map is `docs/FEATURE_MAP.md`; the code map is `docs/CODE_MAP.md`.

## Stack

Tauri 2 (Rust) with React 19, TypeScript, Vite and Zustand. SQLite with FTS5 for the index. Atkinson Hyperlegible Next for the type. Plugins: log, opener, single-instance, autostart, notification.

## Develop

```bash
npm install
npm run dev
```

`npm run dev:ui` runs the renderer alone in a browser with a mocked backend (`src/dev/mockBackend.ts`), which is also what the UI drive uses.

## Verify

```bash
npm run verify
```

Runs typecheck, lint (with the house invariants: zero emoji, zero em or en dash, no silent catch, no committed secret), the text gate over every tracked file, vitest, and the token contrast gate. Then:

```bash
npm run verify:ui
```

drives the renderer with Playwright and writes evidence to `reports/ui-verification/<stamp>/`. The native crate:

```bash
cd src-tauri && cargo test && cargo clippy --all-targets -- -D warnings
```

The real app, on throwaway folders, driven through its own window:

```bash
npm run verify:native
```

needs a release `tote.exe`. `npm run release` makes one (see Release), but it needs the updater key; without the key, build the release configuration that embeds `dist` yourself:

```powershell
npm run build
cd src-tauri
cargo build --release --features tauri/custom-protocol
```

The feature matters: a plain `cargo build --release` makes a dev-mode exe (tauri's build script turns dev on when `custom-protocol` is off) whose window asks `http://localhost:1430` for its page, so the native check stops at the first screen with ERR_CONNECTION_REFUSED. On this checkout, set the `TAURI_CONFIG` override first (see "Building on a path with an apostrophe"). The check seeds `%LOCALAPPDATA%\tote-native-check` with fake messy files, launches the app in test mode, drops downloads with real `Zone.Identifier` streams and checks they stay, sends files with Move to Tote the way File Explorer does (a second `tote.exe --move-to-tote <file>`, handed to the running test app), and checks search, move, rename, pin, Recycle Bin, Undo, Clean up, To do (attachments that follow a move, a restart, the morning reminder, recovery of a damaged list) and Settings against the disk and the database. Evidence lands in `reports/native-check/<stamp>/`.

### Test mode

Two environment variables keep a dry run away from real files:

- `TOTE_ROOTS=<desktop>;<documents>;<downloads>[;<more>...]` replaces the detected folders.
- `TOTE_DATA_DIR=<folder>` keeps the index and settings for that run apart.
- `TOTE_DEBUG_PORT=<port>` (only together with `TOTE_ROOTS`) opens the WebView2 debugging port so the native check can drive the window. A normal start never opens one.

A test-mode run logs to `tote-test.log` and can run beside the installed app (the single-instance rule is off in test mode).

In test mode the app never registers itself to start at login and logs at debug level.

## Installing on her laptop

1. Run `Tote_<version>_x64-setup.exe`. It installs for the current user; no admin password. It also adds **Move to Tote** to File Explorer's right-click menu for her account (on Windows 11 under Show more options, or Shift and right-click); uninstalling removes it.
2. **Set every browser she uses to save straight to Downloads**, so Recent downloads lists every new file for her to send to Tote.
   - Chrome, Edge, Brave: Settings > Downloads > Location: Downloads, and turn off "Ask where to save each file".
   - Firefox: Settings > General > Files and Applications > Save files to: Downloads, and turn off "Always ask you where to save files".
   - Tote checks this itself: Settings > Browsers shows each browser and profile, and the Find screen shows one line while any of them needs a change.
3. In Tote's Settings, confirm the folders, enter the school year and quarter dates, and add her templates.
4. Let her run the five-step walkthrough.

### Moving from Desk

Tote was called Desk until 2026-10-14. On its first start Tote copies Desk's database (pins, templates, settings, Activity and Undo) from `%LOCALAPPDATA%\com.focalstack.desk` and leaves that folder untouched; it copies only while Tote has no database of its own yet. On a laptop that still has Desk, do this in one sitting, in this order:

1. Quit Desk from its tray icon, so two copies never file the same downloads.
2. Install Tote and let it start once. That start makes the copy.
3. Uninstall Desk (Settings > Apps) and leave **"Delete the application data" unticked** in its uninstaller. Ticked, it deletes `com.focalstack.desk`, and if Tote has not started before that, her pins, templates, settings and Undo history are gone for good.

"Start at login" carries over with the settings and is registered for Tote on that start. Desk's old folder stays after its uninstall; delete it by hand once Tote shows her pins and templates.

Downloads a browser saves elsewhere are still noticed (by the browser's web stamp) and listed under Recent downloads, but they stay where they were saved.

### Uninstalling

Settings > Apps > Tote > Uninstall removes the program, its shortcuts and its start at login entry, and never touches her files or `School Files`. Unless "Delete the application data" is ticked in the uninstaller, it leaves behind:

- `%LOCALAPPDATA%\com.focalstack.tote`: the index (`tote.sqlite`, which holds text read from her documents), the To do list (`tasks.sqlite` and its daily copies `tasks.backup.sqlite` and `tasks.backup.previous.sqlite`), any `tote.damaged-*` copies set aside after damage, the log (`logs\tote.log`) and the window's WebView2 data. Ticking the box, or deleting the folder by hand after the uninstall, removes all of it, including her pins, templates, settings, Activity and To do list, for good. Leave it for a reinstall that should keep them.
- a `Tote` value under `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run`, which Windows keeps after the start entry itself is gone. It starts nothing; delete it in the Registry Editor if wanted.
- Desk's old folder `%LOCALAPPDATA%\com.focalstack.desk`, on a laptop that had Desk (see Moving from Desk).

## Release

1. Raise `version` in `src-tauri/tauri.conf.json`, `package.json` and `src-tauri/Cargo.toml` (the same version in all three), and commit. This checkout and the portfolio must both be committed.
2. Build, sign and publish in one step, from PowerShell, with the updater key's passphrase in this terminal only (on this checkout, set the `TAURI_CONFIG` override first, see "Building on a path with an apostrophe"):

   ```powershell
   $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = Read-Host -MaskInput "Updater key passphrase"
   npm run release -- --notes "What changed, in one plain sentence."
   Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD
   ```

   It writes `THIRD-PARTY-NOTICES.txt` (the license notices of every bundled package, installed next to `tote.exe` with `LICENSE.txt`), builds, signs the installer, keeps it and its `.sig` in `installers/`, copies `Tote_<version>_x64-setup.exe` into the portfolio's `tote/download/`, writes `tote/latest.json`, updates the file name, version, size and SHA-256 on the download page, publishes the notices at `tote/third-party-notices.txt`, removes older installers from `tote/download/`, and tags this commit `v<version>`. `--skip-build` publishes the last build after the same checks.
3. In the portfolio folder: review the diff, commit, push, then `vercel deploy --prod`. Then push the tag from this checkout (`git push origin v<version>`). Installed copies offer the update within a day.

### Release preflight

`scripts/release.mjs` checks everything before it writes anything, lists every problem at once and stops with "Nothing was built or written." when there is one (the checks are tested in `scripts/release-checks.test.mjs`, which CI runs):

- `--notes` is given and has no en or em dash.
- `package.json`, `tauri.conf.json` and `Cargo.toml` carry the same version, and it is higher than the one in the published https://www.codewithlex.site/tote/latest.json and in the portfolio's copy (installed copies only take a higher version).
- This checkout and the portfolio have no uncommitted changes, and the tag `v<version>` does not exist yet.
- The download page names exactly one installer, that installer is in `tote/download/`, and the page shows exactly one SHA-256.
- For a build: the updater key exists, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` is not empty, `TAURI_CONFIG` is set on a checkout path with an apostrophe, and the Tauri CLI is installed.

It then builds with the key kept out of the build's environment (the build runs `npm run build` and every package it pulls in), and signs the installer in a step of its own with `tauri signer sign --app-version <version>`, the only process given the key path and the passphrase (`tauri signer sign` reads its `--password` option from `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, so the passphrase is never on a command line). Before any write to the site it verifies the `.sig` the way the updater on her laptop does (made by the trusted key, matching the installer's bytes, its trusted comment naming this version and file), checks that the build changed no tracked file, and works out the page change in memory, changing only the four values that name the linked installer.

### The updater key

The updater key (`%USERPROFILE%\.tauri\tote-updater.key`, or `TOTE_UPDATER_KEY`) is the only thing that lets installed copies trust an update. Never commit it or put it in a synced folder. If it is lost, installed copies can no longer update and each laptop needs one manual install of a build with a new key. A plain `npx tauri build` fails without it, because `createUpdaterArtifacts` is on; the release turns that off for its build and signs afterwards.

The key made for 0.2.0 has no passphrase (audit T-05, 2026-10-16), so the file alone is enough to sign an update, and the release now refuses to sign without one. `npx tauri signer generate` makes a new key and cannot add a passphrase to an existing one, so the plan, for Lex to carry out, is to move installed copies to a new key that has one:

1. Make the new key and type a passphrase at its prompt (not with `--password`, which would land in the shell history), then keep the passphrase in a password manager:

   ```powershell
   npx tauri signer generate -w "$env:USERPROFILE\.tauri\tote-updater-2.key"
   ```

2. Keep the old public key in a file: `tote-updater.key.pub` next to the old key, or the current `plugins.updater.pubkey` value of `src-tauri/tauri.conf.json` saved as a file. Then put the contents of `tote-updater-2.key.pub` in `plugins.updater.pubkey`, raise the version and commit.
3. Publish one release signed with the old key. It still installs on her laptop, and the build it installs trusts the new key from then on:

   ```powershell
   npm run release -- --key-change "$env:USERPROFILE\.tauri\tote-updater.key.pub" --notes "..."
   ```

   `--key-change` signs with the old key (the default `TOTE_UPDATER_KEY`), which has no passphrase, checks the signature against the old public key in that file, and refuses a file that names the key already in `tauri.conf.json`.
4. Deploy, then wait until the foot of Settings on her laptop shows that version.
5. From the next release on, set `$env:TOTE_UPDATER_KEY = "$env:USERPROFILE\.tauri\tote-updater-2.key"` before `npm run release`, and delete the old key and every copy of it: an unencrypted copy stays a usable key until it is gone.

A laptop that missed the key change release (switched off the whole time) refuses every later update, because they are signed with a key it does not know; it needs one manual install from the download page. Back up only the new, passphrase-protected key file, and keep the passphrase apart from it.

### Rolling back

The updater only installs a version higher than the one running, so putting an older `latest.json` back cannot move an updated laptop back, and a build that crashes on start cannot update itself.

- To undo a bad release, publish the last good code under a higher version: start a branch from the last good tag (`git switch -c rollback v<good version>`; releases before the first tagged one are the commits named "Tote 0.x.y: ..." in `git log`) or revert the bad commits, raise the version above the bad one, commit and release as usual. Installed copies take it within a day.
- A laptop whose Tote no longer starts needs one manual install of that fixed, higher version from the download page. Do not install an older installer over a newer one: the newer version may have changed the index's tables.
- Every released installer and its `.sig` stay in `installers/` (kept out of git), for that manual install and to check what was shipped.

The installer is not code-signed, so Windows SmartScreen warns on the first manual install. Updates installed from inside Tote do not show that warning.

### Building on a path with an apostrophe

The Windows resource compiler cannot read an icon path containing `'` (tauri-winres escapes it as `\'`). On this machine the checkout lives at `E:\PROJECTS\Teacher's Desktop Manager`, so native builds point the icon at an apostrophe-free copy through Tauri's config override:

```powershell
Copy-Item src-tauri\icons\* E:\PROJECTS\tote-icons\ -Force
$env:TAURI_CONFIG = '{"bundle":{"icon":["E:/PROJECTS/tote-icons/32x32.png","E:/PROJECTS/tote-icons/128x128.png","E:/PROJECTS/tote-icons/128x128@2x.png","E:/PROJECTS/tote-icons/icon.icns","E:/PROJECTS/tote-icons/icon.ico"]}}'
```

A checkout on a plain path (as in CI) needs none of this. Renaming the folder is the clean fix.

## Safety rules

- Nothing moves on its own. A file moves only when she sends it with Move to Tote or moves it with Move to, Organize or Clean up.
- Nothing is overwritten (`Name (2).docx`), nothing is deleted for good (Recycle Bin only), every change is undoable.
- Every file command validates on the Rust side that the path resolves inside Desktop, Documents, Downloads or OneDrive.
- Failures are logged and shown in Activity in plain words; nothing fails silently.

## License

MIT, see `LICENSE`.
