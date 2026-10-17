// The new-version banner: shown on every screen once the daily check finds a
// newer Tote. One primary action (Update now); Later hides it until the next
// start and Settings keeps a quiet Update meanwhile. The update itself is
// downloaded, verified and installed natively. A failed install says so here
// in the same sentence as its toast, and the button reads Try again.

import { Download } from "lucide-react";
import { useApp } from "../store/app";

export function UpdateBanner() {
  const update = useApp((s) => s.update);
  const dismissed = useApp((s) => s.updateDismissed);
  const updating = useApp((s) => s.updating);
  const updateError = useApp((s) => s.updateError);
  const installUpdate = useApp((s) => s.installUpdate);
  const dismissUpdate = useApp((s) => s.dismissUpdate);
  if (!update || dismissed) return null;

  const note = updating ? "Downloading it now. Tote will close and open again by itself." : updateError ?? "Updating takes about a minute. Tote closes and opens again by itself.";

  return (
    <section className="update-banner" aria-live="polite" aria-labelledby="update-title" data-testid="update-banner">
      <span className="update-icon" aria-hidden>
        <Download size={20} />
      </span>
      <span className="update-text">
        <span className="update-title" id="update-title">
          Tote {update.version} is ready
        </span>
        <span className="update-note" data-testid="update-note">
          {note}
        </span>
      </span>
      <span className="update-actions">
        {!updating && (
          <button type="button" className="button button-quiet" onClick={dismissUpdate} data-testid="update-later">
            Later
          </button>
        )}
        <button type="button" className="button button-primary" onClick={() => void installUpdate()} disabled={updating} data-testid="update-now">
          {updating ? "Updating..." : updateError ? "Try again" : "Update now"}
        </button>
      </span>
    </section>
  );
}
