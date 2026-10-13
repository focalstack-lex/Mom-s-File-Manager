// The window's own title bar: the top strip of the green shell. Dragging it
// moves the window and a double-click maximizes (data-tauri-drag-region);
// the three buttons on the right match the Windows ones. Close goes through
// the normal close request, so Desk still hides to the tray (lib.rs).
// In the plain-browser dev harness there is no window, so only the strip shows.

import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useState } from "react";
import { useApp } from "../store/app";

export function TitleBar() {
  const fail = useApp((s) => s.fail);
  const native = isTauri();
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (!native) return;
    const win = getCurrentWindow();
    const sync = () =>
      win
        .isMaximized()
        .then(setMaximized)
        .catch((error) => console.warn("Desk: could not read the window state", error));
    void sync();
    const off = win.onResized(() => void sync());
    return () => {
      void off.then((stop) => stop()).catch(() => undefined);
    };
  }, [native]);

  const act = (work: () => Promise<void>) => void work().catch(fail);

  return (
    <header className="titlebar" data-tauri-drag-region data-testid="titlebar">
      {native && (
        <div className="window-controls">
          <button type="button" className="window-button" onClick={() => act(() => getCurrentWindow().minimize())} aria-label="Minimize" title="Minimize">
            <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
              <path d="M0 5.5h10" stroke="currentColor" />
            </svg>
          </button>
          <button type="button" className="window-button" onClick={() => act(() => getCurrentWindow().toggleMaximize())} aria-label={maximized ? "Restore" : "Maximize"} title={maximized ? "Restore" : "Maximize"}>
            {maximized ? (
              <svg width="10" height="10" viewBox="0 0 10 10" fill="none" aria-hidden>
                <path d="M2.5 2.5V0.5h7v7h-2" stroke="currentColor" />
                <rect x="0.5" y="2.5" width="7" height="7" stroke="currentColor" />
              </svg>
            ) : (
              <svg width="10" height="10" viewBox="0 0 10 10" fill="none" aria-hidden>
                <rect x="0.5" y="0.5" width="9" height="9" stroke="currentColor" />
              </svg>
            )}
          </button>
          <button type="button" className="window-button is-close" onClick={() => act(() => getCurrentWindow().close())} aria-label="Close" title="Close">
            <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
              <path d="M0.5 0.5l9 9M9.5 0.5l-9 9" stroke="currentColor" />
            </svg>
          </button>
        </div>
      )}
    </header>
  );
}
