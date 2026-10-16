# Code map

Where things live. Read this before changing anything; update it in the same change.

## Native (`src-tauri/src`)

| File | Job |
| --- | --- |
| `lib.rs` | Builds the Tauri app: plugins, database, roots, tray, watchers, scan, command list. Close hides the window. |
| `main.rs` | Entry point; no console window in release. |
| `error.rs` | `ToteError { code, message, path? }`, the one error type every command returns. |
| `paths.rs` | Known folders (`Roots`), `confine` (the boundary check), `validate_name`. |
| `sorter.rs` | `Kind`, `classify`, `stays_in_downloads`, `destination`, `unique_path`. |
| `origin.rs` | `Zone.Identifier` reading and the host-to-label table. |
| `stability.rs` | `Pending`, quiet period, backoff, `can_open_exclusively`. |
| `extract.rs` | Text from docx, xlsx, pptx, pdf, txt, csv with size caps. |
| `dupes.rs` | `group_key` for near-duplicate names. |
| `db.rs` | Schema, open with damage recovery, settings table. |
| `index.rs` | `walk`, `upsert`, `relocate`, `remove`, `search` (`words` and `fts_query` cut typed text like the tokenizer; names rank before contents; `MARK_START` and `MARK_END` wrap snippet matches), `recent_downloads`, pins, `group`. |
| `ops.rs` | Logged operations: move, rename, new folder, recycle, undo, `transfer`. |
| `cleanup.rs` | The three Clean up lists. |
| `organize.rs` | Organize now: grade and subject rules, `destination_for`, `plan` (read from disk), `run` (one batch, lock per file). |
| `browsers.rs` | Reads Chrome, Edge, Brave and Firefox download settings (read only) and says which need a change. |
| `tasks.rs` | The To do list: `tasks.sqlite` attached as schema `todo` (`open_store` with damage recovery from the daily `snapshot`), `purge`, task commands' logic, repeats (`next_due`), the morning `reminder` text. |
| `templates.rs` | Shelf storage and dated copies. |
| `clipboard.rs` | Files on the clipboard (CF_HDROP). |
| `worker.rs` | Downloads watcher and sorting loop, index watchers, initial scan, notification, the 15-minute To do check (`tasks_check`: purge, snapshot, reminder). |
| `tray.rs` | Tray icon and menu, `show`. |
| `update.rs` | Daily update check against codewithlex.site and the signed one-click install. |
| `commands.rs` | Every `#[tauri::command]`, `Settings`, `AppInfo`. |
| `state.rs` | `Core` (db mutex, roots, status) and `AppState`. |
| `update.rs` | Once-a-day update check against codewithlex.site/tote/latest.json, signed download and install; never online in a dry run unless TOTE_UPDATE_VERIFY. |

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
| `lib/browsers.ts` | Browser check labels and the Find notice text. |
| `lib/organize.ts` | Folder labels, plan summary, word lists, folding a run into one Activity entry. |
| `lib/todo.ts` | To do groups (Overdue to No date), due dates in words, repeat words, `hasQuarters`. |
| `lib/paging.ts` | Page sizes, day grouping, `takeRows`, `useShown` (grow on request, reset on a new search). |
| `lib/focus.ts` | Tab trapping for the dialog and walkthrough; arrow-key movement between file rows. |
| `lib/search.ts` | `words` and `hasWord`: typed text cut the way `index::words` cuts names; the Find store and the mock decide with it whether typed text is a search. |
| `store/app.ts` | View, app info, settings save, toasts. |
| `store/find.ts` | Query, chips, results, selection, preview. |
| `store/tour.ts` | Walkthrough steps and state, `placeCard`. |
| `store/todo.ts` | Tasks, the open task, actions with toast Undo (reopen, restore). |
| `components/ResultRow.tsx` | One file row with actions, inline rename, copies; `snippetHtml` turns the snippet marks into `<mark>`. |
| `components/Chips.tsx` | Date chips and More filters. |
| `components/ActionBar.tsx` | Bottom bar for selected files. |
| `components/FolderPicker.tsx` | Move to dialog with New folder. |
| `components/TaskRow.tsx` | One task: checkbox, title, quiet line; details in place (title, due, repeat, files, Delete). |
| `components/AttachDialog.tsx` | Attach a file to a task through a search over the Find index. |
| `components/OrganizeDialog.tsx` | The Organize plan: folders with counts, every file and why, one button. |
| `components/SubjectRules.tsx` | Settings editor for the subject folders and their words. |
| `components/PreviewPanel.tsx` | Quick preview. |
| `components/Tour.tsx` | Walkthrough overlay with mask, ring and arrow. |
| `components/TitleBar.tsx` | The window's own title bar on the green shell: drag region and Windows-style window buttons; no controls in the browser harness. |
| `components/Dialog.tsx`, `Toasts.tsx`, `icons.tsx`, `Skeleton.tsx`, `ShowMore.tsx` | Shared pieces: modal with focus trap, toasts with Undo, kind tiles and the Tote mark, loading rows, the "Showing 10 of 84" foot with Show more. |
| `views/FindView.tsx`, `TodoView.tsx`, `CleanupView.tsx`, `ActivityView.tsx`, `SettingsView.tsx` | The five screens. |
| `dev/mockBackend.ts` | Fake native side for the browser harness and the UI drive; its search matches by word start and marks snippets the way `index.rs` does. |
| `tests/*.test.ts` | vitest for helpers, stores and the text gate. |

## Brand (`brand`)

`tote-logo.svg` is the master of the official two-tone logo (dark green `#143625`, cream `#f2f1df`). `tote-app-icon.svg` puts it on a cream rounded tile; `npx tauri icon brand/tote-app-icon.svg` regenerates every file in `src-tauri/icons` (copy only the tracked ones, and sync `E:\PROJECTS\tote-icons` while the checkout path has an apostrophe). `ToteMark` in `src/components/icons.tsx` is the same geometry in one color, with the cream parts cut out.

## Gates (`scripts`)

`verify-text.mjs` (no emoji, no em or en dash in tracked files), `verify-contrast.mjs` (tokens), `verify-ui.mjs` (Playwright drive of the mocked renderer, evidence in `reports/ui-verification/`), `verify-native.mjs` (the release app on throwaway folders over CDP, evidence in `reports/native-check/`). ESLint invariants live in `eslint-rules/tote-invariants.js`. `release.mjs` builds a signed installer and publishes it to the portfolio; `third-party-notices.mjs` writes THIRD-PARTY-NOTICES.txt from the exact Rust and npm dependency graph (generated at release time, not committed; bundled next to tote.exe with LICENSE.txt).

## Rules learned the hard way

- `Core.db()` returns a guard on a plain `std::sync::Mutex`. Never call anything that locks it again (such as `commands::settings_of`) while a guard is alive; the worker and `get_app_info` both deadlocked that way on 2026-10-12. Read settings first, then lock.
