// The frame: green sidebar, one view at a time, toasts, and the walkthrough
// on first run. Ctrl+F (or /) jumps to the search box from anywhere.

import { BrushCleaning, FolderOpen, History, Search, Settings } from "lucide-react";
import { useEffect } from "react";
import { DeskMark } from "./components/icons";
import { Toasts } from "./components/Toasts";
import { Tour } from "./components/Tour";
import { CHANGED_EVENT, ipc, listen, SORTED_EVENT, type SortedEvent } from "./lib/ipc";
import { useApp, type View } from "./store/app";
import { useFind } from "./store/find";
import { useTour } from "./store/tour";
import { ActivityView } from "./views/ActivityView";
import { CleanupView } from "./views/CleanupView";
import { FindView } from "./views/FindView";
import { SettingsView } from "./views/SettingsView";

const NAV: { id: View; label: string; icon: typeof Search; tour?: string }[] = [
  { id: "find", label: "Find", icon: Search },
  { id: "cleanup", label: "Clean up", icon: BrushCleaning, tour: "nav-cleanup" },
  { id: "activity", label: "Activity", icon: History, tour: "nav-activity" },
];

export default function App() {
  const view = useApp((s) => s.view);
  const setView = useApp((s) => s.setView);
  const info = useApp((s) => s.info);
  const loadInfo = useApp((s) => s.loadInfo);
  const saveSettings = useApp((s) => s.saveSettings);
  const toast = useApp((s) => s.toast);
  const fail = useApp((s) => s.fail);
  const refreshFind = useFind((s) => s.refresh);
  const startTour = useTour((s) => s.start);
  const checkBrowsers = useApp((s) => s.checkBrowsers);

  useEffect(() => {
    void loadInfo();
    void checkBrowsers();
    // Settings can change while Desk sits in the tray; look again when she comes back.
    const onFocus = () => void checkBrowsers();
    window.addEventListener("focus", onFocus);
    const unlisten = [
      listen(CHANGED_EVENT, () => {
        void loadInfo();
        if (useApp.getState().view === "find") void refreshFind();
      }),
      listen<SortedEvent>(SORTED_EVENT, (e) => toast(`${e.name} is in ${e.folder}.`)),
    ];
    return () => {
      window.removeEventListener("focus", onFocus);
      for (const u of unlisten) u.then((fn) => fn()).catch(() => undefined);
    };
  }, [loadInfo, refreshFind, toast, checkBrowsers]);

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

  const finishTour = () => {
    document.querySelector('[data-testid="main"]')?.scrollTo({ top: 0 });
    if (info && !info.settings.tourDone) void saveSettings({ ...info.settings, tourDone: true });
  };

  const openSchoolFiles = () => ipc.openSchoolFiles().catch(fail);

  return (
    <div className="app" data-testid="app">
      <nav className="sidebar" aria-label="Main" data-testid="sidebar">
        <div className="brand">
          <span className="brand-mark" aria-hidden>
            <DeskMark />
          </span>
          <span className="brand-name">Desk</span>
        </div>
        {NAV.map((item) => (
          <button key={item.id} type="button" className={`nav-button${view === item.id ? " is-active" : ""}`} onClick={() => setView(item.id)} aria-current={view === item.id ? "page" : undefined} data-tour={item.tour} data-testid={`nav-${item.id}`} title={item.label}>
            <item.icon size={22} aria-hidden />
            <span className="nav-label">{item.label}</span>
          </button>
        ))}
        <div className="nav-divider" aria-hidden />
        <button type="button" className="nav-button" onClick={() => void openSchoolFiles()} data-testid="nav-folders" title="Open School Files in File Explorer">
          <FolderOpen size={22} aria-hidden />
          <span className="nav-label">Sorted folders</span>
        </button>
        <div className="sidebar-spacer" />
        <button type="button" className={`nav-button${view === "settings" ? " is-active" : ""}`} onClick={() => setView("settings")} aria-current={view === "settings" ? "page" : undefined} data-testid="nav-settings" title="Settings">
          <Settings size={22} aria-hidden />
          <span className="nav-label">Settings</span>
        </button>
      </nav>
      <main className="main" data-testid="main">
        {info === null ? (
          <p className="status-line">Starting Desk...</p>
        ) : view === "find" ? (
          <FindView info={info} />
        ) : view === "cleanup" ? (
          <CleanupView info={info} />
        ) : view === "activity" ? (
          <ActivityView info={info} />
        ) : (
          <SettingsView key={JSON.stringify(info.settings)} info={info} />
        )}
      </main>
      <Toasts />
      <Tour onFinish={finishTour} />
    </div>
  );
}
