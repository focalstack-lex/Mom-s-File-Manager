// The frame: a green shell with the sidebar, the open view on a canvas card,
// toasts, and the walkthrough on first run. Ctrl+F (or /) jumps to the search
// box from anywhere. The sidebar folds into an icon rail; the choice is
// remembered on this computer.

import { BrushCleaning, ChevronLeft, ChevronRight, FolderOpen, History, ListChecks, Search, Settings } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState, type ComponentType } from "react";
import { DropZone } from "./components/DropZone";
import { ToteMark } from "./components/icons";
import { LoadFailed } from "./components/LoadFailed";
import { TitleBar } from "./components/TitleBar";
import { Toasts } from "./components/Toasts";
import { Tour } from "./components/Tour";
import { UpdateBanner } from "./components/UpdateBanner";
import { focusSoon, SEARCH_BOX } from "./lib/focus";
import { labelWords } from "./lib/format";
import { CHANGED_EVENT, ipc, listen, SORTED_EVENT, type SortedEvent } from "./lib/ipc";
import { useApp, type View } from "./store/app";
import { useFind } from "./store/find";
import { useTour } from "./store/tour";
import { ActivityView } from "./views/ActivityView";
import { CleanupView } from "./views/CleanupView";
import { FindView } from "./views/FindView";
import { SettingsView } from "./views/SettingsView";
import { TodoView } from "./views/TodoView";

const NAV: { id: View; label: string; icon: typeof Search; tour?: string }[] = [
  { id: "find", label: "Find", icon: Search },
  { id: "todo", label: "To do", icon: ListChecks, tour: "nav-todo" },
  { id: "cleanup", label: "Clean up", icon: BrushCleaning, tour: "nav-cleanup" },
  { id: "activity", label: "Activity", icon: History, tour: "nav-activity" },
];

/**
 * One menu item. The white label and glass chip are the resting look; the
 * "ink" layer on top is the same icon and label in their active colours, and
 * styles.css clips it to wherever the sliding tab is right now. So the text
 * is dark exactly where the tab is, at every frame of the slide.
 */
function NavItem({ label, icon: Icon, active, onClick, testId, tour, title, badge }: { label: string; icon: ComponentType<{ size?: number }>; active?: boolean; onClick(): void; testId: string; tour?: string; title?: string; badge?: string }) {
  return (
    <button type="button" className={`nav-button${active ? " is-active" : ""}`} onClick={onClick} aria-current={active ? "page" : undefined} data-tour={tour} data-testid={testId} title={title ?? label}>
      <span className="nav-icon" aria-hidden>
        <Icon size={20} />
      </span>
      <span className="nav-label">{label}</span>
      {badge && <span className="nav-new">{badge}</span>}
      <span className="nav-ink" aria-hidden>
        <span className="nav-icon">
          <Icon size={20} />
        </span>
        <span className="nav-label">{label}</span>
      </span>
    </button>
  );
}

const RAIL_KEY = "tote.sidebar.collapsed";

/** Browser storage can be blocked; the sidebar then simply starts open. */
function readCollapsed(): boolean {
  try {
    return window.localStorage.getItem(RAIL_KEY) === "1";
  } catch {
    return false;
  }
}

function writeCollapsed(collapsed: boolean): void {
  try {
    window.localStorage.setItem(RAIL_KEY, collapsed ? "1" : "0");
  } catch (error) {
    console.warn("Tote: could not remember the sidebar choice", error);
  }
}

export default function App() {
  const view = useApp((s) => s.view);
  const setView = useApp((s) => s.setView);
  const info = useApp((s) => s.info);
  const infoError = useApp((s) => s.infoError);
  const loadInfo = useApp((s) => s.loadInfo);
  const saveSettings = useApp((s) => s.saveSettings);
  const toast = useApp((s) => s.toast);
  const fail = useApp((s) => s.fail);
  const refreshFind = useFind((s) => s.refresh);
  const startTour = useTour((s) => s.start);
  const checkBrowsers = useApp((s) => s.checkBrowsers);
  const checkUpdate = useApp((s) => s.checkUpdate);
  const [collapsed, setCollapsed] = useState(readCollapsed);
  const navRef = useRef<HTMLElement>(null);

  // The active tab is one shape that slides to the chosen item. Its place is
  // two animated CSS properties on the sidebar (--tab-y, --tab-h); each item
  // knows its own place (--item-y, --item-h) so its ink layer can follow the
  // tab frame by frame. Measured after layout and whenever the sidebar
  // changes size (folding, a taller window moving Settings down).
  useLayoutEffect(() => {
    const nav = navRef.current;
    if (!nav) return;
    const place = () => {
      for (const item of nav.querySelectorAll<HTMLElement>(".nav-button")) {
        item.style.setProperty("--item-y", `${item.offsetTop}px`);
        item.style.setProperty("--item-h", `${item.offsetHeight}px`);
      }
      const active = nav.querySelector<HTMLElement>('.nav-button[aria-current="page"]');
      nav.dataset.tab = active ? "on" : "off";
      if (active) {
        nav.style.setProperty("--tab-y", `${active.offsetTop}px`);
        nav.style.setProperty("--tab-h", `${active.offsetHeight}px`);
        // In a window short enough to scroll the menu, the last item (Settings)
        // keeps its clearance from the card's rounded corner only with the
        // menu at its end, where the bottom padding is (styles.css, .sidebar).
        const items = nav.querySelectorAll<HTMLElement>(".nav-button");
        if (active === items[items.length - 1]) nav.scrollTop = nav.scrollHeight;
      }
    };
    place();
    // The first placement appears in place; only later moves glide.
    const frame = window.requestAnimationFrame(() => {
      nav.dataset.tabReady = "true";
    });
    const observer = new ResizeObserver(place);
    observer.observe(nav);
    return () => {
      window.cancelAnimationFrame(frame);
      observer.disconnect();
    };
  }, [view, collapsed, info]);

  const toggleSidebar = () => {
    setCollapsed((was) => {
      writeCollapsed(!was);
      return !was;
    });
  };

  useEffect(() => {
    void loadInfo();
    void checkBrowsers();
    void checkUpdate();
    // Settings can change while Tote sits in the tray; look again when she
    // comes back. The update check asks the native side, which goes online
    // at most once a day, so coming back often costs nothing.
    const onFocus = () => {
      void checkBrowsers();
      void checkUpdate();
    };
    window.addEventListener("focus", onFocus);
    // Index changes arrive in bursts (the first scan, a folder being copied);
    // one quiet refresh per burst keeps the screen steady.
    let changeTimer: number | undefined;
    const unlisten = [
      listen(CHANGED_EVENT, () => {
        window.clearTimeout(changeTimer);
        changeTimer = window.setTimeout(() => {
          void loadInfo();
          if (useApp.getState().view === "find") void refreshFind(true);
        }, 400);
      }),
      // "Filed" is the one word for what Move to Tote does, in toasts and in Activity.
      listen<SortedEvent>(SORTED_EVENT, (e) => toast(e.count > 1 ? `Filed ${e.count} files in ${labelWords(e.folder)}.` : `Filed ${e.name} in ${labelWords(e.folder)}.`)),
    ];
    return () => {
      window.removeEventListener("focus", onFocus);
      window.clearTimeout(changeTimer);
      for (const u of unlisten) u.then((fn) => fn()).catch(() => undefined);
    };
  }, [loadInfo, refreshFind, toast, checkBrowsers, checkUpdate]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const typing = event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement;
      const wantsSearch = ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "f") || (event.key === "/" && !typing);
      if (!wantsSearch || document.querySelector('[role="dialog"]')) return;
      event.preventDefault();
      setView("find");
      window.requestAnimationFrame(() => document.querySelector<HTMLInputElement>('[data-testid="search-input"]')?.focus());
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [setView]);

  useEffect(() => {
    if (info && !info.settings.tourDone) startTour();
  }, [info?.settings.tourDone, info, startTour]);

  // A damaged to-do list was replaced on this start (tasks.rs): say so once.
  const restoredShown = useRef(false);
  useEffect(() => {
    const restored = info?.tasksRestored;
    if (!restored || restoredShown.current) return;
    restoredShown.current = true;
    if (restored === "snapshot") toast("Your to-do list was restored from Tote's daily copy. Changes from today may be missing.");
    else toast("Your to-do list could not be read, so it starts empty. The damaged copy was kept.", "error");
  }, [info?.tasksRestored, toast]);

  // tote.sqlite was rebuilt on this start (db.rs): one notice, and whether
  // her pins, templates and settings came along.
  const rebuiltShown = useRef(false);
  useEffect(() => {
    const rebuilt = info?.rebuilt;
    if (!rebuilt || rebuiltShown.current) return;
    rebuiltShown.current = true;
    if (rebuilt.kept) toast("Tote's index had to be rebuilt. Your files were not touched. Your pins, templates and settings were kept.");
    else toast("Tote's index had to be rebuilt. Your files were not touched. Your pins, templates and settings could not be kept. Check Settings.", "error");
  }, [info?.rebuilt, toast]);

  // The walkthrough ends where she starts: at the top of Find, typing in the search box.
  const finishTour = () => {
    document.querySelector('[data-testid="main"]')?.scrollTo({ top: 0 });
    if (info && !info.settings.tourDone) void saveSettings({ ...info.settings, tourDone: true });
    focusSoon(SEARCH_BOX);
  };

  const openSchoolFiles = () => ipc.openSchoolFiles().catch(fail);

  return (
    <div className={`app${collapsed ? " is-collapsed" : ""}`} data-testid="app">
      <TitleBar />
      <nav className="sidebar" aria-label="Main" data-testid="sidebar" ref={navRef}>
        <span className="nav-indicator" aria-hidden data-testid="nav-indicator" />
        <div className="brand">
          <span className="brand-mark" aria-hidden>
            <ToteMark />
          </span>
          <span className="brand-name">Tote</span>
          <button type="button" className="sidebar-toggle" onClick={toggleSidebar} aria-label={collapsed ? "Show menu names" : "Hide menu names"} aria-expanded={!collapsed} title={collapsed ? "Show menu names" : "Hide menu names"} data-testid="sidebar-toggle">
            {collapsed ? <ChevronRight size={16} aria-hidden /> : <ChevronLeft size={16} aria-hidden />}
          </button>
        </div>
        {NAV.map((item) => (
          <NavItem key={item.id} label={item.label} icon={item.icon} active={view === item.id} onClick={() => setView(item.id)} testId={`nav-${item.id}`} tour={item.tour} badge={item.id === "todo" && info && !info.settings.todoSeen ? "New" : undefined} />
        ))}
        <div className="nav-divider" aria-hidden />
        <NavItem label="Sorted folders" icon={FolderOpen} onClick={() => void openSchoolFiles()} testId="nav-folders" title="Open School Files in File Explorer" />
        <div className="sidebar-spacer" />
        <NavItem label="Settings" icon={Settings} active={view === "settings"} onClick={() => setView("settings")} testId="nav-settings" />
      </nav>
      <main className="main" data-testid="main">
        <UpdateBanner />
        {info === null ? (
          infoError ? (
            <LoadFailed text={infoError} onRetry={() => void loadInfo()} testId="app-load-failed" />
          ) : (
            <p className="status-line">Starting Tote...</p>
          )
        ) : view === "find" ? (
          <FindView info={info} />
        ) : view === "todo" ? (
          <TodoView info={info} />
        ) : view === "cleanup" ? (
          <CleanupView info={info} />
        ) : view === "activity" ? (
          <ActivityView info={info} />
        ) : (
          <SettingsView info={info} />
        )}
      </main>
      <Toasts />
      <Tour onFinish={finishTour} />
      <DropZone />
    </div>
  );
}
