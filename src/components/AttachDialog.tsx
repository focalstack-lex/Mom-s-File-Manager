// Attach a file to a task: a search over the same index as Find. Clicking a
// file attaches it and closes the dialog.

import { useEffect, useState } from "react";
import { describeError } from "../lib/errors";
import { locationLine } from "../lib/format";
import { ipc } from "../lib/ipc";
import type { FileRow } from "../lib/types";
import { useTodo } from "../store/todo";
import { Dialog } from "./Dialog";
import { KindIcon } from "./icons";

const LIMIT = 20;

export function AttachDialog({ taskId, roots, onClose }: { taskId: number; roots: string[]; onClose(): void }) {
  const attach = useTodo((s) => s.attach);
  const [query, setQuery] = useState("");
  const [found, setFound] = useState<{ query: string; rows: FileRow[] } | null>(null);
  const [error, setError] = useState<string | null>(null);

  // A short pause after typing, so each word is one search, as on Find.
  useEffect(() => {
    const wanted = query.trim();
    if (wanted === "") return undefined;
    let live = true;
    const timer = window.setTimeout(() => {
      ipc
        .find(wanted, { kinds: [], sources: [] })
        .then((result) => {
          if (!live) return;
          setFound({ query: wanted, rows: result.groups.map((g) => g.latest).slice(0, LIMIT) });
          setError(null);
        })
        .catch((e: unknown) => {
          if (live) setError(describeError(e));
        });
    }, 200);
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [query]);

  const wanted = query.trim();
  const rows = wanted !== "" && found ? found.rows : [];
  const searched = found !== null && found.query === wanted;

  const pick = async (path: string) => {
    if (await attach(taskId, path)) onClose();
  };

  return (
    <Dialog title="Attach a file" onClose={onClose} testId="attach-dialog">
      <div className="field">
        <label htmlFor="attach-search">Search your files</label>
        <input id="attach-search" value={query} onChange={(e) => setQuery(e.target.value)} placeholder="A name, or a word inside the file" autoComplete="off" data-testid="attach-search" />
      </div>
      {rows.length > 0 && (
        <ul className="folder-list" aria-label="Files found">
          {rows.map((row) => (
            <li key={row.path}>
              <button type="button" className="folder-option attach-option" onClick={() => void pick(row.path)} data-testid="attach-option">
                <KindIcon kind={row.kind} size={20} />
                <span className="attach-text">
                  <span className="attach-name">{row.name}</span>
                  <span className="hint">{locationLine(row.parent, roots)}</span>
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
      {searched && rows.length === 0 && <p className="field-note">Nothing found. Try another word.</p>}
      {error && <p className="field-error">{error}</p>}
      <div className="dialog-actions">
        <button type="button" className="button" onClick={onClose}>
          Cancel
        </button>
      </div>
    </Dialog>
  );
}
