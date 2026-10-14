# Tote: To do list design

Status: approved in chat on 2026-10-14 (Lex). Part 1 of two: the To do list ships first; Part 2, the Ask assistant, gets its own spec and builds on this one (it can add a task after she confirms). Main spec: `2026-10-12-teachers-desktop-manager-design.md`.

## 1. Why

Lex asked for productivity features on top of the file manager. School paperwork runs on deadlines that come back (SF2 every month, grades every quarter), and the files for each deadline are already in Tote's index. A short list whose tasks open their files, with one morning reminder, helps her without a new routine beyond typing the task.

## 2. Decisions (Lex, 2026-10-14)

| Decision | Choice |
| --- | --- |
| Where it lives | Its own sidebar item, To do, second after Find. Sidebar becomes Find, To do, Clean up, Activity, (Ask in Part 2), then Settings. Lex asked for the navigation change. |
| Due dates and reminders | Optional due date; one Windows notification on the morning a task is due. This reverses "Deadline reminders" in the main spec's Out of scope (section 4). |
| Attaching files | From inside the task, through a small search that uses the Find index. Find's file rows stay unchanged. |
| Repeats | Never, every week, every month, every quarter (quarter dates from Settings). |
| New marker | The To do item shows a small "New" marker until she opens it once. Amends 5.10 ("no badges") for this one functional marker. |

## 3. The screen

- Sidebar: To do with a checklist icon (lucide `ListChecks`), between Find and Clean up, with the "New" marker described above. A sixth walkthrough step points at it for new installs: "Your to-do list. Write what needs doing. On the morning it is due, Tote reminds you."
- Header: title "To do" and one lead line, like the other views.
- Add row at the top: a text box ("What needs doing? For example: Submit SF2 for October"), an optional Due date picker, and the Add button, which is the screen's one primary action. Enter in the text box adds too. Focus stays in the text box after adding.
- List, in groups: Overdue (heading in coral), Today, Tomorrow, This week (the rest of the Monday to Sunday week), Later, No date. Inside a group: by due date, then oldest first. The list opens on the first 20 tasks and grows with Show more (the 5.10 pattern).
- Each row: a checkbox (at least 40 px), the title, then one quiet line with the due date in words ("today", "tomorrow", "Friday", "17 October"; overdue rows say "was due Friday"), "every month" when it repeats, and "2 files" when files are attached.
- Clicking the row opens its details in place, one task at a time:
  - Title, edited in place (saved on Enter or when she leaves the field).
  - Due date picker, with a way to clear it.
  - Repeat: Never, Every week, Every month, Every quarter. Repeat is disabled without a due date. Every quarter is disabled, with a note pointing to Settings, until at least one quarter has both dates.
  - Attached files: name and kind tile, Open, Show in folder, Remove. A file Tote can no longer find shows "Can't find this file" with Remove.
  - Attach a file: a dialog with a search box over the Find index; clicking a result attaches it and closes the dialog.
  - Delete task.
- Ticking a task off moves it to a folded Done section at the bottom (finished in the last 30 days, heading shows the count). Toast: "Done." with Undo. For a repeating task the toast adds when the next one is due ("Next one is due 14 November.") and the new copy appears in the list.
- Deleting shows a toast with Undo, no confirmation (5.10: undoable instead of confirmed).
- Empty state: "Nothing to do yet. Add your first task above."
- Narrow windows: the add row wraps (text box full width, date and Add below); details stack under the task; no sideways scrolling at 640 px.
- Settings copy: the notifications switch becomes "Windows messages", described as "When a download is filed, and on mornings when a task is due." The walkthrough line says six steps.

## 4. Storage and recovery

- Tasks live in their own file, `tasks.sqlite`, next to `tote.sqlite`, attached to the same connection as schema `todo` (`ATTACH DATABASE ... AS todo`). One connection keeps the existing mutex discipline, and `index::relocate` can update attachments in the same statement batch.
- Why a separate file: a damaged `tote.sqlite` is moved aside and rebuilt from disk (db.rs), which is safe because the index can be rebuilt. Her tasks cannot be rebuilt, so they must never be caught in that rebuild.
- Schema (schema `todo`):
  - `tasks(id INTEGER PRIMARY KEY, title TEXT NOT NULL, due TEXT, repeat TEXT NOT NULL DEFAULT 'none', done_at INTEGER, deleted_at INTEGER, repeated_from INTEGER, edited_at INTEGER, created_at INTEGER NOT NULL)`. `due` is a local date `YYYY-MM-DD`, the same form as the quarters. `repeat` is `none`, `week`, `month` or `quarter`.
  - `task_files(task_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE, path TEXT NOT NULL, added_at INTEGER NOT NULL, PRIMARY KEY(task_id, path))`.
- Daily snapshot: once a day (at start and on the worker's 15-minute check), `VACUUM todo INTO` a temporary file that then replaces `tasks.backup.sqlite`. The day it was taken is kept in the settings table (`tasks_snapshot_on`).
- Opening, before the attach: if `tasks.sqlite` exists and fails to open or fails `PRAGMA integrity_check`, it is renamed `tasks.damaged-<unix time>.sqlite`. Then:
  - a healthy `tasks.backup.sqlite` is copied into place, and Tote shows one toast on start: "Your to-do list was restored from Tote's daily copy. Changes from today may be missing.";
  - with no healthy snapshot, an empty list starts, and the toast says: "Your to-do list could not be read, so it starts empty. The damaged copy was kept." (coral).
- Cleanup on start: tasks deleted more than 7 days ago and tasks done more than 30 days ago are removed for good (their attachment rows go with them).
- Tasks and their titles never leave the laptop.

## 5. Behavior

- Validation (native side): title trimmed, 1 to 200 characters; due is a real `YYYY-MM-DD` date or empty; repeat is one of the four values; repeat other than none needs a due date; quarter needs usable quarters in Settings; attached paths must be inside Tote's roots (the same boundary check as Open) and be files.
- Repeats, computed on the native side when she ticks a repeating task, in one transaction with the tick:
  - Week: due plus 7 days.
  - Month: same day next month; a day the month does not have becomes its last day (31 January to 28 or 29 February). The next copy counts from that date.
  - Quarter: in the quarter that contains the due date, count the days from the due date to the quarter's end; the next copy is due that many days before the end of the next quarter (the one starting soonest after this one ends), never before that quarter's start. A due date outside every quarter goes to the last day of the next quarter that starts after it. With no next quarter, the copy has no due date and the toast says "Add next school year's quarters in Settings to keep it repeating."
  - The next copy keeps the title and repeat, has no files attached, and records `repeated_from`.
- Undo of a tick clears `done_at`. It also removes the copy made by that tick, but only while the copy is untouched: not edited (`edited_at` empty), not done, nothing attached. An edited copy stays.
- Undo of a delete clears `deleted_at`.
- Morning reminder, decided by a pure function of (now, the last day reminded, open tasks, the notifications switch):
  - nothing before 07:00 local time, nothing when the switch is off, nothing when no open task is due today or overdue, at most once a day (`reminded_on` in the settings table);
  - title "To do today"; body lists up to three titles due today, then "and N more", then "Also N overdue."; with only overdue tasks: "1 task is overdue." or "N tasks are overdue.";
  - checked when the worker starts and every 15 minutes, so a laptop first opened at 14:00 is reminded then.
  - Clicking the notification does not open Tote (the Windows limit already noted in the main spec, 4 v1.1).
- Attached files that move: `index::relocate` also rewrites `todo.task_files` paths, as it does for pins and templates, so a move by Tote, background sorting, Organize or Undo keeps the attachment. A file changed outside Tote shows as missing.

## 6. Code

Native (`src-tauri/src`):

- `tasks.rs` (new): types `Task`, `TaskFile`, `Repeat`, `Completed`; `open_store` (damage check, snapshot restore, attach, schema), `snapshot`, `purge`, `list`, `add`, `update`, `complete`, `reopen`, `delete`, `restore`, `attach`, `detach`, `next_due`, `reminder`.
- `db.rs`: `open` attaches the tasks store; `open_memory` attaches an in-memory one for tests.
- `index.rs`: `relocate` updates `todo.task_files`.
- `commands.rs`: `list_tasks`, `add_task`, `update_task`, `complete_task`, `reopen_task`, `delete_task`, `restore_task`, `attach_file`, `detach_file`, all `async` on the blocking pool; `Settings` gains `todo_seen`; `AppInfo` gains `tasks_restored` (`snapshot`, `empty` or null).
- `worker.rs`: the 15-minute check for the reminder and the snapshot. Settings are read before the database lock (the CODE_MAP rule).
- `state.rs`: `Core` carries the tasks restore result.

Renderer (`src`):

- `lib/todo.ts`: grouping, due date words, repeat words, quarter availability.
- `store/todo.ts`: tasks, which task is open, actions that call the native side and toast with Undo.
- `views/TodoView.tsx`, `components/TaskRow.tsx`, `components/AttachDialog.tsx`.
- `App.tsx` (nav item, New marker), `store/app.ts` (`View`, toast Undo for tasks), `components/Toasts.tsx`, `store/tour.ts` (sixth step), `views/SettingsView.tsx` (copy), `lib/ipc.ts`, `lib/types.ts`, `dev/mockBackend.ts`, `styles.css`.

## 7. Testing

- Rust (`cargo test`): every date rule takes "today" as an argument. Weekly; monthly including 31 January, leap years and the year end; quarterly offset, a due date outside every quarter, no next quarter; repeat refused without a due date or without quarters; tick plus copy in one transaction; Undo removes only an untouched copy; delete and restore; purge after 7 and 30 days; title and date validation; reminder before 07:00, once a day, the 14:00 first open, switch off, "and N more", overdue only; damaged store restored from the snapshot; damaged store with no snapshot starts empty and keeps the damaged file; the main database rebuild leaves `tasks.sqlite` alone; `relocate` moves an attachment.
- vitest: grouping on fixed dates including Saturday and Sunday edges, due date words, repeat words, quarter availability.
- UI drive (`npm run verify:ui`, mocked backend): the walkthrough has six steps; To do opens from the sidebar and the New marker goes; add with Enter and with Add; a due date puts the task in its group; tick, toast Undo puts it back; a monthly task makes its next copy and the toast names the date; Every quarter disabled without quarters; Attach a file then Open; delete and Undo; empty state; 640 px without sideways scroll.
- Native check (`npm run verify:native`): a task survives a restart; a damaged `tasks.sqlite` comes back from the snapshot; an attached file moved with Move to stays attached; `reminded_on` is set after the check runs. The notification balloon itself stays a by-eye check, like the download notification.
- Gates before done: `npm run verify:all`, `cargo test`, `cargo clippy -D warnings`, `npm run verify:native`, then the release build. FEATURE_MAP, CODE_MAP (which also gains the missing `update.rs` row), the main spec's amendments and the journal are updated in the same change.
