// The Organize plan, shown before anything moves (spec 5.11): which folders,
// how many files each, and every file if she wants to check. One button runs it.

import { ChevronDown, ChevronRight, Folder } from "lucide-react";
import { useState } from "react";
import { folderLabel, newFolderCount, planSummary } from "../lib/organize";
import type { OrganizePlan } from "../lib/types";
import { Dialog } from "./Dialog";

interface Props {
  plan: OrganizePlan;
  schoolFiles: string;
  busy: boolean;
  onConfirm(): void;
  onClose(): void;
}

const FOLDERS_SHOWN = 8;

export function OrganizeDialog({ plan, schoolFiles, busy, onConfirm, onClose }: Props) {
  const [everyFile, setEveryFile] = useState(false);
  const fresh = newFolderCount(plan);
  const folders = plan.folders.slice(0, FOLDERS_SHOWN);
  return (
    <Dialog title={`Organize ${planSummary(plan.moves.length, plan.folders.length)}?`} onClose={busy ? () => undefined : onClose} testId="organize-dialog">
      <p>
        Grade and subject come from each file name. Files with no clue go by type and month.
        {fresh > 0 && ` ${fresh === 1 ? "1 new folder" : `${fresh} new folders`} will be made in School Files.`}
      </p>
      <ul className="folder-list organize-folders" aria-label="Folders">
        {folders.map((f) => (
          <li key={f.dir} className="organize-folder">
            <Folder size={18} aria-hidden />
            <span className="organize-folder-name">{folderLabel(f.dir, schoolFiles)}</span>
            {!f.exists && <span className="organize-new">new</span>}
            <span className="section-count">{f.count === 1 ? "1 file" : `${f.count} files`}</span>
          </li>
        ))}
        {plan.folders.length > FOLDERS_SHOWN && <li className="organize-more">and {plan.folders.length - FOLDERS_SHOWN} more folders</li>}
      </ul>
      <button type="button" className="button button-quiet align-start" onClick={() => setEveryFile((v) => !v)} aria-expanded={everyFile} data-testid="organize-every-file">
        {everyFile ? <ChevronDown size={18} aria-hidden /> : <ChevronRight size={18} aria-hidden />}
        {everyFile ? "Hide the files" : "See every file"}
      </button>
      {everyFile && (
        <ul className="organize-files" aria-label="Every file and where it goes">
          {plan.moves.map((m) => (
            <li key={m.path}>
              <strong>{m.name}</strong>
              <span>
                to {folderLabel(m.destDir, schoolFiles)} ({m.reason})
              </span>
            </li>
          ))}
        </ul>
      )}
      {plan.skipped > 0 && (
        <p className="field-note">
          {plan.skipped === 1 ? "1 shortcut, installer or zip file stays" : `${plan.skipped} shortcuts, installers and zip files stay`} where {plan.skipped === 1 ? "it is" : "they are"}.
        </p>
      )}
      <p className="field-note">One Undo puts everything back.</p>
      <div className="dialog-actions">
        <button type="button" className="button" onClick={onClose} disabled={busy}>
          Not now
        </button>
        <button type="button" className="button button-primary" onClick={onConfirm} disabled={busy} data-testid="organize-confirm">
          {busy ? "Organizing..." : plan.moves.length === 1 ? "Organize 1 file" : `Organize ${plan.moves.length.toLocaleString()} files`}
        </button>
      </div>
    </Dialog>
  );
}
