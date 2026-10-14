# Tote

Tote finds a teacher's files and keeps new downloads filed, with no new routines to learn. Built for one real user first: Lex's mother, a public school teacher on a Windows laptop whose files are saved everywhere.

What it does:

- Watches Downloads and files every finished download into `Documents\School Files\<Type>\<YYYY-MM>`, remembering where it came from (Messenger, Gmail, Google Drive, DepEd, or the site).
- Indexes Desktop, Documents, Downloads, Pictures, Videos, Music and OneDrive, including the words inside Word, Excel, PowerPoint, PDF and text files, so typing a student's name finds `Book1.xlsx`. Build output inside code projects is skipped.
- Notices downloads a browser saved somewhere else (by the browser's web stamp) and lists them under Recent downloads without moving them, and checks each browser's download settings.
- One Find screen: a big search box, Recent downloads, pinned files, date and type and source filters, copies grouped with the newest marked Latest, a quick preview, and Copy to paste a file into a message.
- Organizing by hand when she wants to: Move to, Rename (extension locked), New folder, Recycle Bin, Pin. Every change is logged in Activity with Undo.
- Clean up: duplicate copies, loose Desktop files and old downloads, each with a suggestion. Nothing moves until she clicks.
- A Templates shelf that makes a dated copy of a master file instead of editing the original.
- Settings: start at login, notifications, school-year dates, templates (native file picker), and the folders Tote looks in (add with a native folder picker, remove all but Documents and Downloads; changes apply after a restart).
- A five-step walkthrough with arrows on first run, replayable from Settings.
- No network access at all.

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

needs a release `tote.exe` (see Release). It seeds `%LOCALAPPDATA%\tote-native-check` with fake messy files, launches the app in test mode, drops downloads with real `Zone.Identifier` streams, and checks filing, search, move, rename, pin, Recycle Bin, Undo, Clean up and Settings against the disk and the database. Evidence lands in `reports/native-check/<stamp>/`.

### Test mode

Two environment variables keep a dry run away from real files:

- `TOTE_ROOTS=<desktop>;<documents>;<downloads>[;<more>...]` replaces the detected folders.
- `TOTE_DATA_DIR=<folder>` keeps the index and settings for that run apart.
- `TOTE_DEBUG_PORT=<port>` (only together with `TOTE_ROOTS`) opens the WebView2 debugging port so the native check can drive the window. A normal start never opens one.

A test-mode run logs to `tote-test.log` and can run beside the installed app (the single-instance rule is off in test mode).

In test mode the app never registers itself to start at login and logs at debug level.

## Installing on her laptop

1. Run `Tote_<version>_x64-setup.exe`. It installs for the current user; no admin password.
2. **Set every browser she uses to save straight to Downloads.** Tote files only what lands there.
   - Chrome, Edge, Brave: Settings > Downloads > Location: Downloads, and turn off "Ask where to save each file".
   - Firefox: Settings > General > Files and Applications > Save files to: Downloads, and turn off "Always ask you where to save files".
   - Tote checks this itself: Settings > Browsers shows each browser and profile, and the Find screen shows one line while any of them needs a change.
3. In Tote's Settings, confirm the folders, enter the school year and quarter dates, and add her templates.
4. Let her run the five-step walkthrough.

### Moving from Desk

Tote was called Desk until 2026-10-14. On a laptop that still has Desk, quit Desk from its tray icon and uninstall it (Settings > Apps) before installing Tote, so two copies never file the same downloads. On its first start Tote copies Desk's database (pins, templates, settings, Activity and Undo) from `%LOCALAPPDATA%\com.focalstack.desk`, and leaves that folder untouched. "Start at login" carries over with the settings and is registered for Tote on that start.

Downloads a browser saves elsewhere are still noticed (by the browser's web stamp) and listed under Recent downloads, but they stay where they were saved.

## Release

```bash
npm run build:release
```

produces an NSIS installer under `src-tauri/target/release/bundle/nsis/`. The installer is unsigned in v1, so Windows SmartScreen warns on first install.

### Building on a path with an apostrophe

The Windows resource compiler cannot read an icon path containing `'` (tauri-winres escapes it as `\'`). On this machine the checkout lives at `E:\PROJECTS\Teacher's Desktop Manager`, so native builds point the icon at an apostrophe-free copy through Tauri's config override:

```powershell
Copy-Item src-tauri\icons\* E:\PROJECTS\tote-icons\ -Force
$env:TAURI_CONFIG = '{"bundle":{"icon":["E:/PROJECTS/tote-icons/32x32.png","E:/PROJECTS/tote-icons/128x128.png","E:/PROJECTS/tote-icons/128x128@2x.png","E:/PROJECTS/tote-icons/icon.icns","E:/PROJECTS/tote-icons/icon.ico"]}}'
```

A checkout on a plain path (as in CI) needs none of this. Renaming the folder is the clean fix.

## Safety rules

- Only new downloads move automatically. Existing files are indexed, never moved, unless she moves them.
- Nothing is overwritten (`Name (2).docx`), nothing is deleted for good (Recycle Bin only), every change is undoable.
- Every file command validates on the Rust side that the path resolves inside Desktop, Documents, Downloads or OneDrive.
- Failures are logged and shown in Activity in plain words; nothing fails silently.

## License

MIT, see `LICENSE`.
