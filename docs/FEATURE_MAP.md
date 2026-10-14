# Feature map

Every surface a user can reach, where it lives, and which verify step proves it. Update this file in the same change as the surface.

| Surface | Spec | Renderer | Native | Verify step |
| --- | --- | --- | --- | --- |
| Walkthrough (first run, arrows, Enter and Escape) | 5.9 | `src/components/Tour.tsx`, `src/store/tour.ts` | settings `tourDone` | UI drive 1, 2, 13, native 2 |
| Find: search box (Ctrl+F or / from anywhere), status line, result count, searching state | 5.3 | `src/views/FindView.tsx`, `src/App.tsx` | `commands::find`, `index::search` | UI drive 3, 4, native 3 |
| Find: Recent downloads with source and location | 5.3 | `src/components/ResultRow.tsx` | `index::recent_downloads`, `origin` | UI drive 3, 14, native 4 |
| Find: Pinned and orphan pins | 5.4 | `src/views/FindView.tsx` | `index::pinned`, `index::orphan_pins` | UI drive 3 |
| Find: copies grouped with Latest | 5.3 | `src/components/ResultRow.tsx` | `dupes`, `index::group` | UI drive 5 |
| Find: date chips, More filters (type, source) | 5.3 | `src/components/Chips.tsx`, `src/lib/dates.ts` | `index::Filters` | UI drive 6, vitest dates |
| Find: quick preview (drawer under 900 px, Escape closes, labelled Open, Show in folder, Copy) | 5.3 | `src/components/PreviewPanel.tsx` | `commands::preview`, `extract` | UI drive 7 |
| Row actions: Open, Show in folder, Copy, Pin (shown on hover, focus, selection or preview); click previews, double-click or Enter opens, arrow keys move | 5.3 | `src/components/ResultRow.tsx`, `src/lib/focus.ts` | `commands::open_path`, `reveal_path`, `copy_files`, `set_pinned` | UI drive 7, native 7 |
| Action bar: Move to (folder picker, New folder) | 5.4 | `src/components/ActionBar.tsx`, `FolderPicker.tsx` | `ops::move_file`, `ops::create_folder`, `commands::list_destinations` | UI drive 8, native 5 |
| Action bar: Rename inline, extension locked | 5.4 | `src/components/ResultRow.tsx` | `ops::rename_file`, `paths::validate_name` | UI drive 9, native 6 |
| Action bar: Recycle Bin with confirmation listing the files | 5.4 | `src/components/ActionBar.tsx`, `src/components/Dialog.tsx` | `ops::recycle` (trash crate) | UI drive 10, native 8 |
| Templates shelf | 5.6 | `src/views/FindView.tsx`, `SettingsView.tsx` | `templates` | cargo test templates |
| Activity with Undo, grouped by day; several files moved or recycled at once are one row with Undo all; filed and moved rows marked green with Show in folder; quiet Undo that outlines on hover; failures in coral with Show the file | 5.7 | `src/views/ActivityView.tsx`, `src/lib/organize.ts` (`batchTitle`) | `ops::list`, `ops::undo`, `ops::undo_batch`, `ops::new_batch` (Move and Recycle of several files) | UI drive 11, 11b, native 8, cargo test ops |
| Clean up: duplicates, Desktop clutter, old downloads as divided rows of one card, count pills, chevron at the right (an extra copy is only offered for the Recycle Bin) | 5.5 | `src/views/CleanupView.tsx` | `cleanup::report` | UI drive 12, native 9 |
| Settings: start at login and notifications (switches), school year and quarters (date pickers), replay | 5.8 | `src/views/SettingsView.tsx` | `commands::set_settings`, autostart plugin | UI drive 13, native 10 |
| Settings: folders Tote looks in (add via native picker, remove, restart note) | 5.8 | `src/views/SettingsView.tsx` | `commands::add_folder`, `remove_folder`, `restart_app`, dialog plugin | UI drive 13b |
| Settings: templates via native file picker | 5.6 | `src/views/SettingsView.tsx` | `commands::add_template`, dialog plugin | UI drive 13c |
| Test mode (TOTE_ROOTS, TOTE_DATA_DIR) | 8 | none | `paths::Roots::from_env`, `lib.rs` | native 1 |
| New version banner and one-click update; Settings switch "Check for new versions" | 6.6 amendment | `src/components/UpdateBanner.tsx`, `src/views/SettingsView.tsx` | `commands::check_update`, `install_update`, `update.rs`, updater plugin | cargo test (update), UI drive 16, native update check |
| Title bar: the shell's top strip (no Windows frame), drag to move, double-click to maximize, Minimize, Maximize or Restore, Close (hides to the tray) | 7 | `src/components/TitleBar.tsx`, `src/styles.css` | `lib.rs` window builder (`decorations(false)`, `shadow(true)`), window permissions in `capabilities/default.json` | native 12 |
| Sidebar: green shell, glass menu pills, active item as a canvas tab that slides between items (ink layer keeps the label dark under it), views ease in, folds to an icon rail (remembered) | 7 | `src/App.tsx`, `src/styles.css` | none | UI drive 14b |
| Sorted folders (opens School Files) | 5.1 | `src/App.tsx` | `commands::open_school_files` | manual |
| Background sorting, origin, notification | 5.1 | toast on `tote://sorted` | `worker`, `stability`, `sorter`, `origin` | cargo test, UI drive 14 (event), native 4 |
| Index scan and live updates, build folders skipped | 5.2 | status line | `worker::start_scan`, `index::walk`, `index::prune_except` | cargo test index, native 2b |
| Downloads saved outside Downloads (web stamp) | 5.1.3 | Recent downloads | `index::note_if_downloaded`, `origin::read_zone` | cargo test index, native 2c |
| Browser check (Find notice, Settings > Browsers) | 5.1.2 | `src/lib/browsers.ts`, `FindView.tsx`, `SettingsView.tsx` | `browsers`, `commands::browser_check` | cargo test browsers, vitest browsers, UI drive 13a |
| Undo in the toast after Move, Recycle Bin and Clean up moves | 5.7 | `src/components/Toasts.tsx`, `src/store/app.ts` (`moveOpIds`) | `ops::undo`, `ops::list` | UI drive 10 (toast shows Undo) |
| Long lists open short: Recent downloads by day (10), results (20), folded Clean up cards (10), Activity (Today and Yesterday) | 5.10 | `src/lib/paging.ts`, `src/components/ShowMore.tsx`, `FindView.tsx`, `CleanupView.tsx`, `ActivityView.tsx` | none | vitest paging, UI drive 3b, 12b |
| Organize now: plan dialog, run, toast Undo, one Activity row with Undo all; Settings subject words | 5.11 | `src/components/OrganizeDialog.tsx`, `CleanupView.tsx`, `ActivityView.tsx`, `src/components/SubjectRules.tsx`, `src/lib/organize.ts` | `organize`, `ops::undo_batch`, `commands::organize_plan`, `organize_run`, `undo_batch` | cargo test organize, vitest organize, UI drive 12c and 13d, native 8b |
| Tray icon, close hides, Quit | 5.1 | none | `tray` | manual |
| Narrow windows (rail at 900 px, preview drawer, two-line Clean up rows, no overflow to 640 px) | 7 | `src/styles.css` | none | UI drive 15 |

"native N" is a step of `npm run verify:native` (scripts/verify-native.mjs), which drives the release app on throwaway folders through its own window. The tray icon and the Windows notification balloon are the two things still checked by eye.
