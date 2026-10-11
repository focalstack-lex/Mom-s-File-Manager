# Code map

Where things live. Read this before changing anything; update it in the same change.

## Native (`src-tauri/src`)

| File | Job |
| --- | --- |
| `lib.rs` | Builds the Tauri app: plugins, database, roots, tray, watchers, scan, command list. Close hides the window. |
| `main.rs` | Entry point; no console window in release. |
| `error.rs` | `DeskError { code, message, path? }`, the one error type every command returns. |
| `paths.rs` | Known folders (`Roots`), `confine` (the boundary check), `validate_name`. |
| `sorter.rs` | `Kind`, `classify`, `stays_in_downloads`, `destination`, `unique_path`. |
| `origin.rs` | `Zone.Identifier` reading and the host-to-label table. |
| `stability.rs` | `Pending`, quiet period, backoff, `can_open_exclusively`. |
| `extract.rs` | Text from docx, xlsx, pptx, pdf, txt, csv with size caps. |
| `dupes.rs` | `group_key` for near-duplicate names. |
| `db.rs` | Schema, open with damage recovery, settings table. |
| `index.rs` | `walk`, `upsert`, `relocate`, `remove`, `search`, `recent_downloads`, pins, `group`. |
| `ops.rs` | Logged operations: move, rename, new folder, recycle, undo, `transfer`. |
| `cleanup.rs` | The three Clean up lists. |
| `templates.rs` | Shelf storage and dated copies. |
| `clipboard.rs` | Files on the clipboard (CF_HDROP). |
| `worker.rs` | Downloads watcher and sorting loop, index watchers, initial scan, notification. |
| `tray.rs` | Tray icon and menu, `show`. |
| `commands.rs` | Every `#[tauri::command]`, `Settings`, `AppInfo`. |
| `state.rs` | `Core` (db mutex, roots, status) and `AppState`. |

## Renderer (`src`)

| File | Job |
| --- | --- |
| `main.tsx` | Boot; installs the mock backend in a plain browser during dev. |
| `App.tsx` | Sidebar, view switch, events, walkthrough trigger. |
| `styles.css` | Tokens (contrast-gated), layout, components, responsive rules. |
| `lib/ipc.ts` | Typed command calls and event listening; routes to the mock when present. |
| `lib/types.ts` | Shapes shared with the Rust structs. |
| `lib/format.ts` | Relative times, location words, sizes, stems. |
| `lib/dates.ts` | Date chips and spans, quarter and school year. |
| `lib/errors.ts` | Error code to plain sentence. |
| `store/app.ts` | View, app info, settings save, toasts. |
| `store/find.ts` | Query, chips, results, selection, preview. |
| `store/tour.ts` | Walkthrough steps and state, `placeCard`. |
| `components/ResultRow.tsx` | One file row with actions, inline rename, copies. |
| `components/Chips.tsx` | Date chips and More filters. |
| `components/ActionBar.tsx` | Bottom bar for selected files. |
| `components/FolderPicker.tsx` | Move to dialog with New folder. |
| `components/PreviewPanel.tsx` | Quick preview. |
| `components/Tour.tsx` | Walkthrough overlay with mask, ring and arrow. |
| `components/Dialog.tsx`, `Toasts.tsx`, `icons.tsx` | Shared pieces. |
| `views/FindView.tsx`, `CleanupView.tsx`, `ActivityView.tsx`, `SettingsView.tsx` | The four screens. |
| `dev/mockBackend.ts` | Fake native side for the browser harness and the UI drive. |
| `tests/*.test.ts` | vitest for helpers, stores and the text gate. |

## Gates (`scripts`)

`verify-text.mjs` (no emoji, no em or en dash in tracked files), `verify-contrast.mjs` (tokens), `verify-ui.mjs` (Playwright drive, evidence in `reports/ui-verification/`). ESLint invariants live in `eslint-rules/desk-invariants.js`.
