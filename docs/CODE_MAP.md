# Code map

Where things live. Read this before changing anything; update it in the same change.

## Native (`src-tauri/src`)

| File | Job |
| --- | --- |
| `lib.rs` | Builds the Tauri app: plugins, database, roots, tray, watchers, scan, command list. Close hides the window. Start at login is applied only when settings survived a database rebuild. A database that cannot be opened at start is one plain sentence in a Windows message box (`show_start_failure`; log only in a dry run) before the exit. The app is built then run with a hook that writes the Downloads run marker (`worker::mark_run`) on `RunEvent::Exit`. |
| `main.rs` | Entry point; no console window in release. |
| `error.rs` | `ToteError { code, message, path? }`, the one error type every command returns; `message` is always one plain sentence with a next step and raw OS or library text goes to the log (`From<std::io::Error>` maps the kind through `io_words`, sharing violations 32 and 33 to `inUse`; `From<rusqlite::Error>` says a read failed, `db_write` and `as_write` say a save failed); the codes include `inUse`, `notInRecycleBin`, `noRecycleBin`, `updateSignature` and `updateInstall` (2026-10-16); `FILE_GONE` and `NAME_TAKEN` are the one sentence each for a missing file and a taken name, and a test scans every handwritten `ToteError::new` and `ToteError::validation` sentence in ops, commands, clipboard, tasks, templates, paths, update, worker and lib for a next step (a short named list of instructions such as "Type a name first." count as their own next step). |
| `paths.rs` | Known folders (`Roots`), `confine` (the boundary check), `validate_name`. |
| `sorter.rs` | `Kind` (`words` for a log line: "a Word file"), `classify`, `stays_in_downloads` (installers, archives, scripts, shortcuts and saved web pages: exe, msi, bat, cmd, ps1, vbs, js, jse, wsf, hta, lnk, url, scr, com, reg, html, htm), `describe`, `destination`, `unique_path`. |
| `origin.rs` | `Zone.Identifier` reading and the host-to-label table. |
| `stability.rs` | `Pending` (a size change also brings `next_try` forward to the end of the quiet period), quiet period, backoff, `has_partial_sibling` (an `X.pdf.part` or `X.pdf.crdownload` beside the file means the browser is still writing), `can_open_exclusively`. |
| `extract.rs` | Text from docx, xlsx, pptx, pdf, txt, csv with size caps. |
| `dupes.rs` | `group_key` for near-duplicate names. |
| `db.rs` | Schema, open with damage recovery (`set_aside` moves the main file with its -wal and -shm under one stamp; a -wal another program holds is copied beside the kept copy and left, a held -shm is logged and left; when the fresh file then cannot be opened, `put_back` returns the damaged copy under its own name so the next start reruns the rebuild with nothing lost; `salvage` copies settings, pins, templates and ops out of the damaged copy one table per statement, `Rebuilt { kept }` reports it), settings table, `migrate` (columns added later: `ops.batch`, `ops.size`, `ops.recycled_at`; a one-time rewrite of apostrophe names in `files_fts` under the `fts_name_forms` marker). |
| `index.rs` | `walk`, `upsert`, `relocate`, `remove`, `search` (`words` and `fts_query` cut typed text like the tokenizer; `indexed_name` stores a name with an apostrophe with and without it; names rank before contents; a wordless box browses with the chips; `MARK_START` and `MARK_END` wrap snippet matches), `recent_downloads`, pins, `group`; every write goes through `run`, so a failed pin, relocate or removal says not saved. |
| `ops.rs` | Logged operations: move, rename, new folder, recycle (`recycle_refusal`: no Recycle Bin on a drive that is not fixed, through winsafe `GetDriveType`, or for a path of 260 or more UTF-16 units; each row is finished right after its delete, so a later target that fails leaves the earlier rows recorded; the Recycle Bin is listed once after the deletes and only a row it contradicts is marked failed; the row records the size and the moment), undo (`pick_restore` matches a Recycle Bin item by path, recorded moment and size; the shell is given the full name at `landing_for`, the recorded path or "Name (2).docx" when another file has taken it, and `check_restored` judges that landing only, so a file already at the path is never checked or sent back; `undo`, `undo_batch` and `restore_from_recycle_bin` take the `Placed` registry and `note_placed` records a landing in Downloads before the file moves, so the watcher leaves it alone; `was_put_back` says whether an undone row names a path, for the watcher's start-up sweep), `transfer`. A failed Recycle Bin call is worded by `recycle_failed` (a file held open reads as `inUse`); every ops write goes through `write`, so a failure says not saved. Log lines name the kind of file (`sorter::describe`), the path only at Debug. |
| `cleanup.rs` | The three Clean up lists. |
| `organize.rs` | Organize now: grade and subject rules, `destination_for`, `plan` (read from disk), `run` (one batch, lock per file). |
| `browsers.rs` | Reads Chrome, Edge, Brave and Firefox download settings (read only) and says which need a change. |
| `tasks.rs` | The To do list: `tasks.sqlite` attached as schema `todo` (`open_store` with damage recovery from the daily `snapshot`, a missing file restored the same way after any stale -wal or -shm is set aside; `usable_backup` falls back to `tasks.backup.previous.sqlite` when the latest copy is missing or unreadable; `snapshot` keeps the copy it replaces as the previous one, leaving the latest in place until the new copy has its name, and never replaces either generation's tasks with an empty list), `purge`, task commands' logic (every write goes through `write`, so a failed save says not saved), repeats (`next_due`), the morning `reminder` text. |
| `templates.rs` | Shelf storage and dated copies. |
| `clipboard.rs` | Files on the clipboard (CF_HDROP). |
| `worker.rs` | Downloads watcher and sorting loop, index watchers, initial scan, notification, the 15-minute To do check (`tasks_check`: purge, snapshot, reminder). The Downloads promise (audit T-06, T-07, T-15, T-51): `is_new_download(created, stamped, run_marker, now)` is the pure rule (created after the previous run's marker, or stamped and created within 10 minutes); `start_run_marker` reads the previous run's `LAST_RUN_KEY` setting (now on the first run) and writes this run's, `mark_run` refreshes it every 15 minutes and at exit; `unseen_downloads` feeds stamped files that arrived while Tote was closed to the loop at start, skipping any path an Undo put back (`ops::was_put_back`); a candidate `core.placed` covers is dropped; `tick` keeps an empty file or one with a `.part` or `.crdownload` sibling waiting and drops it quietly after a day; `sort_one` files into the month the file was created (`created_local`). Info lines name the kind of file (`sorter::describe`, `Kind::words`), never its name; the path is at Debug (T-52). |
| `tray.rs` | Tray icon and menu, `show`. |
| `update.rs` | Daily update check against codewithlex.site and the signed one-click install. |
| `commands.rs` | Every `#[tauri::command]`, `Settings`, `AppInfo` (`rebuilt`, `tasks_unavailable`); `check_folder_choice` (against `covered_folders`: the roots Tote started with plus the folders added this session) and `choose_folder` behind `add_folder`; `settings_from` reads settings one field at a time (`read_settings`, `field`), resets only a field it cannot read, and keeps the text aside under `settings.unreadable-<time>` when it did; `use_template` returns `TemplateCopy { path, opened }`; `blocking` and `log_failure` put every failed command in the log, and `opener_failed` words a failed Open or Show in folder. |
| `state.rs` | `Core` (db mutex, roots, status, `rebuilt`, `tasks_unavailable`, `placed`), `Placed` (paths Tote itself just put in Downloads, keyed without case, covered for `PLACED_FOR` of 10 minutes on every event and forgotten after; its own small mutex, never held while anything else is locked) and `AppState`. |
| `update.rs` | Once-a-day update check against codewithlex.site/tote/latest.json, signed download and install; never online in a dry run unless TOTE_UPDATE_VERIFY. A failure is logged with the library's text and worded by its `Stage` (`words_for`): a failed check or download is `network`, a signature that does not check out `updateSignature`, a failed install `updateInstall`. |

## Renderer (`src`)

| File | Job |
| --- | --- |
| `main.tsx` | Boot; installs the mock backend in a plain browser during dev. |
| `App.tsx` | Sidebar, view switch, events, walkthrough trigger, the start-up notices (to-do restore, index rebuild). |
| `styles.css` | Tokens (contrast-gated), layout, components, responsive rules. |
| `lib/ipc.ts` | Typed command calls and event listening; routes to the mock when present. `useTemplate` answers `{ path, opened }` (2026-10-16). |
| `lib/types.ts` | Shapes shared with the Rust structs. |
| `lib/format.ts` | Relative times, location words, sizes, stems. |
| `lib/dates.ts` | Date chips and spans, quarter and school year. |
| `lib/errors.ts` | `describeError`: the native sentence when it is plain, else `CODE_SENTENCES[code]`; `isPlain` and `plainMessage` keep program text (os error, Error during, debug dumps) off the screen; `recycleFailureText` words a Recycle Bin action that did not fully succeed; `undoText` words an Undo that only partly worked ("Put back 5 of 6. X: reason"), shared by the toast Undo and Activity. |
| `lib/browsers.ts` | Browser check labels and the Find notice text. |
| `lib/organize.ts` | Folder labels, plan summary, word lists, folding a run into one Activity entry; `batchTitle` counts only the rows that went ("Moved 1 of 2 files to the Recycle Bin"). |
| `lib/todo.ts` | To do groups (Overdue to No date), due dates in words, repeat words, `hasQuarters`. |
| `lib/paging.ts` | Page sizes, day grouping, `takeRows`, `useShown` (grow on request, reset on a new search). |
| `lib/focus.ts` | Tab trapping for the dialog and walkthrough; arrow-key movement between file rows. |
| `lib/search.ts` | `words` and `hasWord`: typed text cut the way `index::words` cuts names; the Find store and the mock decide with it whether typed text is a search. |
| `store/app.ts` | View, app info (`infoError` when the first load failed: the shell shows it with Try again), settings save (returns whether it was kept), toasts. |
| `store/find.ts` | Query, chips, results, selection, preview; `failed` keeps the last answer on screen when a search fails. |
| `store/tour.ts` | Walkthrough steps and state, `placeCard`. |
| `store/todo.ts` | Tasks (`loadError` when a load failed), the open task, actions with toast Undo (`reopen` and `restore` return whether the task is back). |
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
| `components/Dialog.tsx`, `Toasts.tsx`, `icons.tsx`, `Skeleton.tsx`, `ShowMore.tsx`, `LoadFailed.tsx` | Shared pieces: modal with focus trap, toasts with Undo (the Undo tries every part, names what stayed through `undoText`, and says "Back on your list." only when the task is back), kind tiles and the Tote mark, loading rows, the "Showing 10 of 84" foot with Show more, the one-line "could not load" notice with Try again. |
| `views/FindView.tsx`, `TodoView.tsx`, `CleanupView.tsx`, `ActivityView.tsx`, `SettingsView.tsx` | The five screens. |
| `dev/mockBackend.ts` | Fake native side for the browser harness and the UI drive; its search matches by word start and marks snippets the way `index.rs` does; edge states come from the query string (`?rebuilt=kept`, `?rebuilt=lost`, `?tasks=unavailable`, `?template=unopened`); `window.__toteMockFail = { command, error, times? }` makes the next call of that command reject (`times` calls, default 1; a mount load runs twice under StrictMode in the harness, so the drive plans 2 and clears the rest before Try again); `undo_batch` names a file that is no longer where the run put it and leaves its row done, as the native side does, and `window.__toteMock.vanish(path)` removes a file as if it were deleted in File Explorer. |
| `tests/*.test.ts` | vitest for helpers, stores and the text gate. |

## Brand (`brand`)

`tote-logo.svg` is the master of the official two-tone logo (dark green `#143625`, cream `#f2f1df`). `tote-app-icon.svg` puts it on a cream rounded tile; `npx tauri icon brand/tote-app-icon.svg` regenerates every file in `src-tauri/icons` (copy only the tracked ones, and sync `E:\PROJECTS\tote-icons` while the checkout path has an apostrophe). `ToteMark` in `src/components/icons.tsx` is the same geometry in one color, with the cream parts cut out.

## Gates (`scripts`)

`verify-text.mjs` (no emoji, no em or en dash in tracked files), `verify-contrast.mjs` (tokens), `verify-ui.mjs` (Playwright drive of the mocked renderer, evidence in `reports/ui-verification/`), `verify-native.mjs` (the release app on throwaway folders over CDP, evidence in `reports/native-check/`). ESLint invariants live in `eslint-rules/tote-invariants.js`. `release.mjs` builds a signed installer and publishes it to the portfolio; `third-party-notices.mjs` writes THIRD-PARTY-NOTICES.txt from the exact Rust and npm dependency graph (generated at release time, not committed; bundled next to tote.exe with LICENSE.txt).

## Rules learned the hard way

- `Core.db()` returns a guard on a plain `std::sync::Mutex`. Never call anything that locks it again (such as `commands::settings_of`) while a guard is alive; the worker and `get_app_info` both deadlocked that way on 2026-10-12. Read settings first, then lock.
- `unsafe_code` is denied, and every `windows-sys` function is `unsafe`: a Win32 call goes through a safe wrapper crate with a documented reason (winsafe for `GetDriveType`), never through `windows-sys` directly.
- SQLite folds a WAL it can checkpoint into the file and deletes it when the last connection closes, so a damaged database's -wal has usually gone before `db::open` sets the file aside; `set_aside` still moves -wal and -shm for the case where the checkpoint failed (the test marks them read-only to get there).
- The Recycle Bin crate reports success where Windows deleted for good (a non-fixed drive, a path of 260 or more units), so `ops::recycle` refuses those first and reads the bin afterwards instead of trusting the call.
- `ToteError.message` is what she reads: one plain sentence with a next step, never OS or library text. The `From` impls in `error.rs` do that for io and SQLite errors; a `map_err` on a plugin or crate error writes its own `log::warn!` line with the raw text first, then builds the sentence. A `format!("...: {e}")` in a message is the bug the 2026-10-16 audit found (T-17).
