// Move to... (spec 5.4): her folders, recent destinations first, with New folder.
// Focus lands on the first folder once the list arrives, so Enter or Space
// picks it straight away.

import { Folder, FolderPlus } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { ipc } from "../lib/ipc";
import { describeError } from "../lib/errors";
import type { Destination } from "../lib/types";
import { Dialog } from "./Dialog";

interface Props {
  count: number;
  warnOld: boolean;
  onPick(destination: string): void;
  onClose(): void;
}

export function FolderPicker({ count, warnOld, onPick, onClose }: Props) {
  const [folders, setFolders] = useState<Destination[]>([]);
  const [chosen, setChosen] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const listRef = useRef<HTMLUListElement>(null);
  const listFocused = useRef(false);

  useEffect(() => {
    ipc.destinations().then(setFolders).catch((e) => setError(describeError(e)));
  }, []);

  // Once, when the folders first arrive: the dialog opened on New folder
  // because the list was still empty then.
  useEffect(() => {
    if (listFocused.current || folders.length === 0) return;
    listFocused.current = true;
    const dialog = listRef.current?.closest<HTMLElement>(".dialog");
    const active = document.activeElement;
    if (dialog && (active === document.body || dialog.contains(active))) listRef.current?.querySelector<HTMLElement>(".folder-option")?.focus();
  }, [folders]);

  const create = async () => {
    const parent = chosen ?? folders[0]?.path;
    if (!parent) return;
    try {
      const path = await ipc.createFolder(parent, newName.trim());
      const label = `${folders.find((f) => f.path === parent)?.label ?? ""} > ${newName.trim()}`;
      setFolders((list) => [{ path, label, recent: false }, ...list]);
      setChosen(path);
      setCreating(false);
      setNewName("");
      setError(null);
    } catch (e) {
      setError(describeError(e));
    }
  };

  return (
    <Dialog title={count === 1 ? "Move this file to" : `Move ${count} files to`} onClose={onClose} testId="folder-picker">
      {warnOld && <p>{count === 1 ? "Word's Recent list will lose this file. Find will still show it." : "Word's Recent list will lose these files. Find will still show them."}</p>}
      <ul className="folder-list" ref={listRef}>
        {folders.map((folder) => (
          <li key={folder.path}>
            <button type="button" className="folder-option" aria-pressed={chosen === folder.path} onClick={() => setChosen(folder.path)}>
              <Folder size={20} aria-hidden />
              {folder.label}
              {folder.recent && <span className="hint">recent</span>}
            </button>
          </li>
        ))}
      </ul>
      {creating ? (
        <div className="field">
          <label htmlFor="new-folder-name">New folder inside {folders.find((f) => f.path === (chosen ?? folders[0]?.path))?.label ?? "the chosen folder"}</label>
          <input id="new-folder-name" autoFocus value={newName} onChange={(e) => setNewName(e.target.value)} onKeyDown={(e) => e.key === "Enter" && void create()} />
        </div>
      ) : (
        <button type="button" className="button button-quiet align-start" onClick={() => setCreating(true)}>
          <FolderPlus size={18} aria-hidden />
          New folder
        </button>
      )}
      {error && <p className="field-error">{error}</p>}
      <div className="dialog-actions">
        <button type="button" className="button" onClick={onClose}>
          Cancel
        </button>
        {creating ? (
          <button type="button" className="button button-primary" onClick={() => void create()} disabled={newName.trim() === ""}>
            Create folder
          </button>
        ) : (
          <button type="button" className="button button-primary" onClick={() => chosen && onPick(chosen)} disabled={!chosen} data-testid="move-confirm">
            Move here
          </button>
        )}
      </div>
    </Dialog>
  );
}
