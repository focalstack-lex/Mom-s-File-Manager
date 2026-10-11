// Move to... (spec 5.4): her folders, recent destinations first, with New folder.

import { Folder, FolderPlus } from "lucide-react";
import { useEffect, useState } from "react";
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

  useEffect(() => {
    ipc.destinations().then(setFolders).catch((e) => setError(describeError(e)));
  }, []);

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
      {warnOld && <p>Word's Recent list will lose this file. Search here will still find it.</p>}
      <ul className="folder-list">
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
        <button type="button" className="button button-quiet" onClick={() => setCreating(true)} style={{ alignSelf: "flex-start" }}>
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
