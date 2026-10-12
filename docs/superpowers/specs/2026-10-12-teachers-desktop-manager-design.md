# Teacher's Desktop Manager: v1 design

Status: approved in chat on 2026-10-12, implementation started the same day. Product name on screen: "Desk" (working name, easy to say, no trademark check yet). Repository: https://github.com/focalstack-lex/Mom-s-File-Manager

## 1. Problem and user

One real user: Lex's mother, a public school teacher using a Windows laptop. Her files are saved everywhere. She cannot find:

- files she just downloaded (from Messenger, Gmail, DepEd and school sites), because she does not know where they landed;
- older files she remembers only vaguely ("that Grade 7 quiz from last month");
- the right copy among near-duplicates (`Lesson Plan (3).docx`, `final_final.docx`).

Hard constraints:

- Zero new routines. Everything that keeps her files in order happens in the background. She only opens one screen to find things. Organizing by hand is offered, never required.
- Nothing overwhelming. One thing happening on screen at a time. A first-run walkthrough with arrows shows her the four things that matter, once.

## 2. Success criteria

1. She finds a file she downloaded today in under 10 seconds, without help, by opening the app.
2. She finds an older file by typing a word she remembers from its name or its contents.
3. Given several copies of a document, she opens the most recently edited one without guessing.
4. No file is ever lost: every move is logged and undoable, nothing is permanently deleted.
5. Her existing habits keep working: old files stay where they are unless she moves them herself.
6. After the walkthrough she can say what the search box, Recent downloads, Open and Undo do.

## 3. Decisions log

| Decision | Choice | Reason |
| --- | --- | --- |
| Platform | Windows only | Her laptop; Windows stamps download origin, which we read. |
| Approach | Tauri 2 desktop app | Full control over sorting and search; reuses the Filewell stack; can later be offered to other teachers. Rejected: Python script plus Task Scheduler (leaves search to Windows search, which is what fails her now); no-code tools such as Everything plus a sorter (name-only search, two tools to learn). |
| What may move automatically | New downloads only | Moving years of old files breaks Word's Recent list and shortcuts she relies on. |
| Sort rule | By type, then month | Predictable and never wrong. Rejected: keyword rules for grade and subject (misfiles), sorting by source (groups by arrival, not content). |
| Manual organizing | Basic actions, pins, cleanup helper | Rejected: full Explorer-style folder browser (duplicates File Explorer, clutters her screen). |
| UI language | Plain English labels | Taglish labels remain an option after watching her use it. |
| Visual direction | Forest-green sidebar, pale canvas, white cards, from Lex's reference image | Adjusted by the binding design checklist (section 7). |
| First run | Guided walkthrough with arrows, five steps, skippable, replayable from Settings | Requested by Lex 2026-10-12. |
| Folder changes | Add and remove in Settings through a native picker; they apply after a restart | The watchers and the scan are built once at startup; a restart button keeps it simple and predictable. Documents and Downloads cannot be removed because filing depends on them. |
| Dry runs | `DESK_ROOTS` and `DESK_DATA_DIR` environment overrides, no autostart in that mode | Lets the real app be exercised on copied folders without touching anything real (section 8). |
| Downloads saved outside Downloads | Noticed by the browser's web stamp, listed in Recent downloads, never moved; plus a browser settings check | Found on 2026-10-12 when Lex's own test download went to a Videos subfolder and Desk never saw it. Moving a file she saved on purpose would override her choice. |
| Pictures, Videos, Music | Indexed and watched, never sorted | Downloads often land there through Save As; old photos and videos must not move. |
| Build folders | Skipped only beside a project file | Lex's Documents held 32,208 indexed files, mostly Rust build output; a teacher's own folder named "build" must still be found. |
| Retry schedule | 5 s, 5 s, 10 s, 20 s, 60 s, 2 min, then 15 min | Fresh downloads are held briefly by the antivirus; the first retries are quick. Changed from 5 s, 30 s, 2 min on 2026-10-12. |

## 4. Scope

### v1

- Background download sorting with source detection and a notification (5.1).
- Search index over her folders with full-text extraction (5.2).
- Find screen: search, recent downloads, pins, filters, duplicate grouping, quick preview, copy to send (5.3).
- Organizing: move, rename, new folder, Recycle Bin, pins (5.4).
- Clean up: duplicate copies, Desktop clutter, old downloads (5.5).
- Templates shelf (5.6).
- Activity log with undo (5.7).
- Settings with school-year dates (5.8).
- First-run walkthrough (5.9).

### v1.1 (designed later, kept here so v1 does not block them)

- Dragging a row out of the window into Messenger or Gmail (v1 has the Copy button).
- Notification click opens the file (the Tauri notification plugin has no click action on Windows; v1 notifications are informational).
- Flash drive helper: "Copy to flash drive" on any file when a USB drive is plugged in; the drive is searchable while connected.
- Weekly backup of `School Files` to OneDrive or an external drive, one-way, never deleting from the backup.
- Text recognition on photos of documents using the Windows built-in OCR.
- Clean up card "Big files": old videos and installers taking space.

### Later

- Taglish search synonyms (`pagsusulit` finds `quiz`), built from her real search history.
- Convert to PDF button.

### Out of scope

Deadline reminders, automatic version history, phone sync, Teacher's Assistant pet integration, network features of any kind.

## 5. Behavior

### 5.1 Download sorting

- The app starts at login (autostart) and lives in the system tray. Closing the window hides it; Quit is in the tray menu. Only one instance runs.
- It watches the user's Downloads folder (resolved through the known-folder API, not a hardcoded path), top level only.
- Ignored: partial downloads (`.crdownload`, `.part`, `.tmp`, `.download`), hidden and system files, folders, and files already being processed.
- A file is ready when its size has not changed for 5 seconds and it can be opened with no sharing (no other program holds it). If it is not ready, the app retries with backoff (5 s, 30 s, 2 min, then every 15 min for 24 hours). If she opened the file straight away in Word, it is sorted after she closes it.
- Before moving, the app reads the file's origin (5.1.1) and stores it in the database.
- Destination: `Documents\School Files\<Type>\<YYYY-MM>\`, where month is the download time.

| Type folder | Extensions |
| --- | --- |
| Word | .doc .docx .rtf .odt |
| PDF | .pdf |
| Excel | .xls .xlsx .csv .ods |
| PowerPoint | .ppt .pptx .odp |
| Images | .jpg .jpeg .png .gif .heic .webp .bmp |
| Videos | .mp4 .mov .avi .mkv .wmv |
| Other | everything else except the exclusions below |

- Not moved, left in Downloads: installers and scripts (`.exe`, `.msi`, `.bat`, `.cmd`, `.ps1`), and archives (`.zip`, `.rar`, `.7z`), because these are run or extracted once.
- Name clash: never overwrite. Append ` (2)`, ` (3)`, and so on before the extension.
- Same volume: a rename. Different volume: copy, verify size, then remove the source; if verification fails, the copy is removed and the original stays.
- On any failure the file stays exactly where it was and the failure appears in Activity.
- After a successful sort, a Windows notification: "Grade 7 Quiz 2.docx saved to School Files > Word."

#### 5.1.1 Origin labels

Browsers on Windows write a `Zone.Identifier` alternate data stream on downloads, holding `HostUrl` and often `ReferrerUrl`. The app reads it and maps the host to a label:

| Host contains | Label |
| --- | --- |
| messenger.com, facebook.com, fbcdn.net, fbsbx.com | Messenger |
| mail.google.com, mail-attachment.googleusercontent.com | Gmail |
| drive.google.com, docs.google.com, drive.usercontent.google.com | Google Drive |
| deped.gov.ph | DepEd |
| anything else | the bare domain, shown as "from example.com" |
| stream missing | "Unknown source" |

Limits stated in the UI where relevant: the origin tells which site, never which person sent it. Files saved by apps that do not write the stream show "Unknown source". The mapping table lives in one module so it can be extended.

#### 5.1.2 Browser check

Desk reads each browser's own settings file, read only: Chrome, Edge and Brave (`Preferences` in every profile) and Firefox (`prefs.js`). A browser that asks where to save each file, or saves somewhere other than Downloads, gets a coral mark and one sentence saying where to change it in Settings > Browsers. While any browser needs a change, the Find screen shows one quiet line with a "How to fix" button. The check runs at start and whenever the window gets focus. Setting her browsers is also step one of the installation checklist in the README.

#### 5.1.3 Downloads saved elsewhere

A browser stamps every download with `Zone.Identifier` (Internet zone 3 or untrusted zone 4), wherever it saves it. A stamped file that appears in any watched folder within 10 minutes of being created counts as a download: it joins Recent downloads with its source, and it stays where she saved it. Only files that land in Downloads are moved. On the first scan, stamped files created in the last 30 days are marked the same way, so Recent downloads is useful from the first minute.

### 5.2 Search index

- Indexed roots: Desktop, Documents, Downloads, Pictures, Videos, Music, and OneDrive when present. All resolved through known-folder APIs. Pictures, Videos and Music are searched and watched for downloads, never sorted.
- Skipped: hidden and system files and folders, `AppData`, `node_modules`, `.git`, `__pycache__`, Python `venv` folders, build output folders (`target`, `dist`, `build`, `out`, `bin`, `obj`) only when they sit beside a project file such as `Cargo.toml` or `package.json`, and files over 50 MB for content extraction (still indexed by name). Rows left from files or folders that are now skipped are pruned at the end of each scan.
- Content extraction for `.docx`, `.xlsx`, `.pptx` (zip archives of XML), `.pdf`, `.txt`, `.csv`. Older `.doc`, `.xls`, `.ppt` are indexed by name only in v1. Each extraction has a size limit; a file that fails is indexed by name only and never blocks the rest.
- Initial scan runs on a low-priority background thread, newest files first. Progress shows as one quiet line on the Find screen until done.
- After the scan, a recursive watcher on each root keeps the index current, debounced.
- Storage: one SQLite database in the app data folder, using FTS5. Stored per file: path, name, extension, size, modified, origin label, origin host, downloaded-at, extracted text, duplicate group key.

### 5.3 Find screen

- One large search box at the top, focused when the window opens. Searches names and contents as she types (debounced). Name matches rank above content matches; ties break by recency. Content matches show a short snippet.
- Empty search box shows, in order: Pinned files, then Recent downloads (newest first, last 30 days).
- Each result row: file-type icon, name, origin and relative time ("from Messenger, Tuesday"), current location in plain words ("in School Files > Word > October"), and the actions Open, Show in folder, Copy.
- Filter chips: date (Today, This week, This month, This quarter, School year, All), type (Word, PDF, Excel, PowerPoint, Images), source (Messenger, Gmail, Google Drive, DepEd, Other). Quarter and school-year chips appear only after their dates are set (5.8).
- Duplicate grouping: names are normalized (lowercase, extension kept separate, and the patterns ` (n)`, `copy`, `copy of`, `final`, `revised`, `v<n>` and runs of separators removed). Files sharing a normalized name and extension form one group. The group shows its most recently modified member with a green **Latest** label and "3 other copies" that expand.
- Quick preview: selecting a row opens a side panel showing the extracted text (first 40 lines) for documents, the image for images, and file details for everything else.
- Copy: places the file itself on the clipboard (as a file, not its path) so she can paste into a Messenger or Gmail message.

### 5.4 Organizing

- Every row has a checkbox. Selecting one or more shows a bottom bar: **Move to...**, **Rename** (single selection), **Pin**, **Recycle Bin**.
- Move to: a picker listing `School Files` folders and her top-level folders in the indexed roots, recent destinations first, with **New folder**. Moving a file that is not a fresh download shows one line: "Word's Recent list will lose this file. Search here will still find it."
- Rename: inline. The extension is locked. Characters Windows forbids (`\ / : * ? " < > |`) and reserved names are rejected as she types, with a plain message. A clash asks before acting.
- Recycle Bin: a confirmation states how many files. Files go to the Windows Recycle Bin, never deleted permanently.
- Boundaries: every operation validates on the Rust side that source and destination resolve (canonicalized) inside the indexed roots. Anything outside is refused. Folders can be created but not deleted in v1.
- Pins: a pinned file shows at the top of Find. Pins follow files moved through the app. A pinned file moved or deleted outside the app shows "Moved or deleted outside the app" with a Search for it button.

### 5.5 Clean up

A sidebar view with three cards. Nothing happens without her click; no destructive box is ticked by default.

1. Duplicate copies: each group with its Latest member marked. She ticks extra copies to send to the Recycle Bin.
2. Desktop clutter: loose files on the Desktop, each with a suggested destination using the 5.1 type-and-month rule (month from the file's modified date). Per file: accept, change folder, or skip. An "Accept all" button acts only on rows still showing a suggestion.
3. Old downloads: files in Downloads not modified for 60 days. Actions: move into School Files, or Recycle Bin.

### 5.6 Templates shelf

- She (or Lex during setup) adds master files to a Templates shelf on the Find screen.
- Clicking a template creates a copy named `<Template name> <YYYY-MM-DD>.<ext>` in `School Files\<Type>\<YYYY-MM>\`, clash rule applied, then opens it. The master file is never opened for editing from the shelf.

### 5.7 Activity and undo

- Every automatic sort, move, rename, folder creation and Recycle Bin action is written to an operations log before it runs, then marked done or failed.
- Activity lists them newest first in plain words, with Undo on each completed one. Undo of a move or rename restores the original path (clash rule applied if the original path is now taken). Undo of Recycle Bin restores the item from the Recycle Bin when Windows allows it; otherwise the entry says to open the Recycle Bin.
- Failures show in coral with an icon and the reason in plain words.

### 5.8 Settings

Minimal: start at login (on by default), notifications (on by default), indexed roots (defaults above, add or remove), school year start and end dates, four quarter date ranges, and "Show the walkthrough again". No other settings in v1.

### 5.9 First-run walkthrough

- Runs once, the first time the window opens, before anything else. Skippable at any step with one "Skip" link; replayable from Settings.
- Five steps, each a short card with an arrow drawn to the real control it describes:
  1. Search box: "Type anything you remember. A name, a student, a word inside the file."
  2. Recent downloads: "Everything you download lands here, newest first. It is already filed for you."
  3. The Open button on the first row: "Open it. Or show it in its folder."
  4. Clean up in the sidebar: "When you have time, this helps tidy up. Nothing moves until you click."
  5. Activity in the sidebar: "Every change is listed here. Undo puts it back."
- Each step has Next, Back and Skip. The rest of the screen dims; the target control stays bright with a soft ring. Keyboard: Enter for Next, Escape to skip.
- The walkthrough uses sample rows when the index is still empty, labelled "Sample" so nothing looks real.

### 5.10 Simplicity rules (apply to every screen)

- One primary action per screen. Secondary actions appear only when a file is selected.
- No counters, charts, badges or tiles beyond one quiet status line.
- Settings stay behind a sidebar footer link, never on the Find screen.
- Confirmations only for the Recycle Bin and for moving old files. Everything else is undoable instead of confirmed.
- Error messages are one sentence in plain words, with what to do next.
- Long lists open short and grow on request ("show a little, offer more"), with a "Showing 10 of 84" line and one "Show N more" button:
  - Recent downloads: the first 10, under day headings (Today, Yesterday, Earlier this week, Earlier).
  - Search results: the first 20 copy groups; a new search or filter starts again at 20.
  - Clean up: a card with more than 5 entries starts folded, its heading showing the count; opened, it shows 10 at a time. "Move all N as suggested" names how many files it acts on, including those not yet shown.
  - Activity: Today and Yesterday first (between 10 and 30 rows), then 20 older at a time.
  - Background refreshes keep what she has opened; nothing collapses while the scan runs.

## 6. Architecture

### 6.1 Stack

Same as Filewell (`E:\PROJECTS\Desktop File Manager`): Tauri 2, Rust, React 19, TypeScript, Vite, Zustand, lucide-react icons, vitest, Playwright for the UI drive. Tauri plugins: log, opener, single-instance, autostart, notification.

### 6.2 Rust modules

| Module | Job |
| --- | --- |
| `paths` | Known-folder resolution, canonicalization, the "inside indexed roots" check used by every command. |
| `watcher` | Downloads watcher (top level) and index watchers (recursive), debounced, feeding a worker thread. |
| `stability` | Decides when a download is ready (size stable, openable with no sharing), owns the retry schedule. |
| `origin` | Reads `Zone.Identifier`, parses `HostUrl` and `ReferrerUrl`, maps to a label. |
| `sorter` | Type classification, destination path, clash naming. |
| `ops` | Move, rename, new folder, Recycle Bin, undo. Writes the operations log before acting. Used by the sorter and by the UI. |
| `extract` | Text from OOXML, PDF and plain text, with size limits. |
| `index` | SQLite schema, initial scan, incremental updates, search queries and ranking. |
| `dupes` | Name normalization and Latest selection. Pure functions. |
| `cleanup` | Builds the three Clean up lists from the index. |
| `templates` | Shelf storage and dated copies. |
| `clipboard` | Puts files on the clipboard as files. |
| `commands` | Thin Tauri command layer: validates input, calls modules, maps errors to plain messages. |

### 6.3 Data flow

1. Download lands, `watcher` emits, `stability` waits, `origin` reads the label, `sorter` asks `ops` to move (logged), `index` records the file with its origin, notification fires, the Find screen refreshes through an event.
2. She types, the UI calls `search`, `index` returns ranked rows grouped by `dupes`.
3. She organizes, the UI calls `ops` through `commands`, `index` updates from the result, Activity shows the entry.

### 6.4 React side

Zustand stores: files (search, recent, pins), selection, activity, settings, tour. Views: Find, Clean up, Activity, Settings. "Sorted folders" in the sidebar opens `School Files` in File Explorer. Shared components: result row, filter chip, bottom action bar, folder picker, preview panel, walkthrough overlay.

### 6.5 Error handling

- No silent failures: every failure is logged to a rotating log file and, when it affects her, shown in Activity in plain words. Raw errors never reach the UI.
- Fail closed: an operation whose paths cannot be validated is refused.
- The database is the only state besides her files; if it is damaged, the app rebuilds the index from disk.

### 6.6 Security and privacy

- No network access at all: no telemetry, no updates over the network in v1, strict CSP, minimal Tauri capabilities.
- All file paths from the UI are validated on the Rust side. The renderer never gets general filesystem access.
- No secrets exist in this app. Releases still pass the Security-First Deployment Gate before installing on her laptop.

## 7. Visual design

From Lex's reference image, checked against `C:/Users/User/Documents/Severus/docs/DESIGN_REFERENCE.md`.

- Kept: deep forest-green sidebar, pale grey-green canvas, white rounded cards with soft shadows, a single green accent, generous negative space.
- Green is the one brand hue: sidebar, primary button, selected chip, Latest label. Everything else is neutral.
- Color is information only: coral only for failures needing attention; each source gets one muted tone; states always carry an icon and text, never color alone.
- No decorative dashboard tiles, charts or promo cards. One quiet text line is allowed: "12 downloads sorted this week".
- Typeface: Atkinson Hyperlegible Next (SIL Open Font License), bundled through the `@fontsource/atkinson-hyperlegible-next` package. Body 16 to 17 px, file names 18 px semibold, search box 22 px.
- Every text and background pair passes WCAG AA on its actual surface, enforced by `npm run verify:contrast`. Click targets at least 44 px.
- Window layout: sidebar plus main area; under 900 px wide the sidebar collapses to an icon rail; no horizontal scrolling at any width down to 360 px.
- House rules: no emoji, no em or en dashes anywhere in UI copy, code or docs; no eyebrow pills above headings.

## 8. Testing

- Rust unit tests: clash naming, type classification, origin parsing and mapping, stability decisions, duplicate normalization and Latest selection, extraction from generated fixtures, path boundary checks, undo of each operation kind.
- Rust integration test: a temporary Downloads folder and temporary roots filled with fake messy files; run sorting, indexing, search and undo end to end. Her real folders are never used in development.
- Renderer: vitest for store logic and helpers.
- Verify scripts adapted from Filewell: typecheck, lint, text check (no emoji, no em or en dash), contrast, and the UI drive with mocked IPC (`verify:ui`), evidence under `reports/ui-verification/`.
- Before installing on her laptop: a dry run on a copy of a few of her messy folders, then the release build through the deployment gate.

## 9. Delivery

- Release as an NSIS installer built with `npx tauri build`. The installer is unsigned in v1, so Windows SmartScreen will warn on first install; Lex installs it in person.
- First-run setup by Lex on her laptop: confirm indexed roots, enter school year and quarter dates, add her templates. Then the walkthrough runs for her.
- Watch her use it for one or two weeks, note what she types and where she hesitates, then plan v1.1.

## 10. Risks

- Drive E: on Lex's machine runs low on space during Tauri builds. Keep `cargo clean` in mind.
- `Zone.Identifier` coverage varies by app and browser; "Unknown source" must look normal, not like an error.
- PDF text extraction quality varies; scanned PDFs have no text until v1.1 OCR.
- OneDrive "files on demand" placeholders must not be downloaded by indexing; content extraction skips files not present locally.
