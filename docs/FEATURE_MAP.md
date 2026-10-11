# Feature map

Every surface a user can reach, where it lives, and which verify step proves it. Update this file in the same change as the surface.

| Surface | Spec | Renderer | Native | Verify step |
| --- | --- | --- | --- | --- |
| Walkthrough (first run, arrows, Enter and Escape) | 5.9 | `src/components/Tour.tsx`, `src/store/tour.ts` | settings `tourDone` | UI drive 1, 2, 13, native 2 |
| Find: search box, status line | 5.3 | `src/views/FindView.tsx` | `commands::find`, `index::search` | UI drive 3, 4, native 3 |
| Find: Recent downloads with source and location | 5.3 | `src/components/ResultRow.tsx` | `index::recent_downloads`, `origin` | UI drive 3, 14, native 4 |
| Find: Pinned and orphan pins | 5.4 | `src/views/FindView.tsx` | `index::pinned`, `index::orphan_pins` | UI drive 3 |
| Find: copies grouped with Latest | 5.3 | `src/components/ResultRow.tsx` | `dupes`, `index::group` | UI drive 5 |
| Find: date chips, More filters (type, source) | 5.3 | `src/components/Chips.tsx`, `src/lib/dates.ts` | `index::Filters` | UI drive 6, vitest dates |
| Find: quick preview | 5.3 | `src/components/PreviewPanel.tsx` | `commands::preview`, `extract` | UI drive 7 |
| Row actions: Open, Show in folder, Copy, Pin | 5.3 | `src/components/ResultRow.tsx` | `commands::open_path`, `reveal_path`, `copy_files`, `set_pinned` | UI drive 7, native 7 |
| Action bar: Move to (folder picker, New folder) | 5.4 | `src/components/ActionBar.tsx`, `FolderPicker.tsx` | `ops::move_file`, `ops::create_folder`, `commands::list_destinations` | UI drive 8, native 5 |
| Action bar: Rename inline, extension locked | 5.4 | `src/components/ResultRow.tsx` | `ops::rename_file`, `paths::validate_name` | UI drive 9, native 6 |
| Action bar: Recycle Bin with confirmation | 5.4 | `src/components/ActionBar.tsx` | `ops::recycle` (trash crate) | UI drive 10, native 8 |
| Templates shelf | 5.6 | `src/views/FindView.tsx`, `SettingsView.tsx` | `templates` | cargo test templates |
| Activity with Undo, failures in coral | 5.7 | `src/views/ActivityView.tsx` | `ops::list`, `ops::undo` | UI drive 11, native 8 |
| Clean up: duplicates, Desktop clutter, old downloads | 5.5 | `src/views/CleanupView.tsx` | `cleanup::report` | UI drive 12, native 9 |
| Settings: start at login, notifications, school year, replay | 5.8 | `src/views/SettingsView.tsx` | `commands::set_settings`, autostart plugin | UI drive 13, native 10 |
| Settings: folders Desk looks in (add via native picker, remove, restart note) | 5.8 | `src/views/SettingsView.tsx` | `commands::add_folder`, `remove_folder`, `restart_app`, dialog plugin | UI drive 13b |
| Settings: templates via native file picker | 5.6 | `src/views/SettingsView.tsx` | `commands::add_template`, dialog plugin | UI drive 13c |
| Test mode (DESK_ROOTS, DESK_DATA_DIR) | 8 | none | `paths::Roots::from_env`, `lib.rs` | native 1 |
| Sorted folders (opens School Files) | 5.1 | `src/App.tsx` | `commands::open_school_files` | manual |
| Background sorting, origin, notification | 5.1 | toast on `desk://sorted` | `worker`, `stability`, `sorter`, `origin` | cargo test, UI drive 14 (event), native 4 |
| Index scan and live updates, build folders skipped | 5.2 | status line | `worker::start_scan`, `index::walk`, `index::prune_except` | cargo test index, native 2b |
| Downloads saved outside Downloads (web stamp) | 5.1.3 | Recent downloads | `index::note_if_downloaded`, `origin::read_zone` | cargo test index, native 2c |
| Browser check (Find notice, Settings > Browsers) | 5.1.2 | `src/lib/browsers.ts`, `FindView.tsx`, `SettingsView.tsx` | `browsers`, `commands::browser_check` | cargo test browsers, vitest browsers, UI drive 13a |
| Tray icon, close hides, Quit | 5.1 | none | `tray` | manual |
| Narrow windows (rail at 900 px, no overflow to 640 px) | 7 | `src/styles.css` | none | UI drive 15 |

"native N" is a step of `npm run verify:native` (scripts/verify-native.mjs), which drives the release app on throwaway folders through its own window. The tray icon and the Windows notification balloon are the two things still checked by eye.
