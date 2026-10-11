// One file on the Find screen: name, where it came from, where it is now,
// Open and Show in folder always, Copy and Pin as icon buttons.

import { Check, Copy, FolderOpen, Pin, PinOff, X } from "lucide-react";
import { useState, type KeyboardEvent } from "react";
import { ipc } from "../lib/ipc";
import { copiesLabel, extOf, locationLine, originLine, stemOf } from "../lib/format";
import type { FileRow } from "../lib/types";
import { useApp } from "../store/app";
import { useFind } from "../store/find";
import { KindIcon } from "./icons";

interface Props {
  file: FileRow;
  roots: string[];
  latest?: boolean;
  others?: FileRow[];
  sample?: boolean;
  tourOpen?: boolean;
  renaming?: boolean;
  onRenamed?(): void;
  onCancelRename?(): void;
}

export function ResultRow({ file, roots, latest, others = [], sample, tourOpen, renaming, onRenamed, onCancelRename }: Props) {
  const toast = useApp((s) => s.toast);
  const fail = useApp((s) => s.fail);
  const selected = useFind((s) => s.selected.includes(file.path));
  const toggleSelected = useFind((s) => s.toggleSelected);
  const previewPath = useFind((s) => s.previewPath);
  const setPreview = useFind((s) => s.setPreview);
  const refresh = useFind((s) => s.refresh);
  const [showCopies, setShowCopies] = useState(false);
  const [stem, setStem] = useState(stemOf(file.name));
  const [renameError, setRenameError] = useState<string | null>(null);

  const run = async (action: () => Promise<unknown>, done?: string) => {
    if (sample) return;
    try {
      await action();
      if (done) toast(done);
    } catch (error) {
      fail(error);
    }
  };

  const submitRename = async () => {
    if (stem.trim() === stemOf(file.name)) {
      onCancelRename?.();
      return;
    }
    try {
      await ipc.renameFile(file.path, stem.trim());
      setRenameError(null);
      onRenamed?.();
      await refresh();
      toast(`Renamed to ${stem.trim()}${extOf(file.name)}`);
    } catch (error) {
      setRenameError(error && typeof error === "object" && "message" in error ? String((error as { message: string }).message) : "That name will not work.");
    }
  };

  const onRenameKey = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter") void submitRename();
    if (event.key === "Escape") onCancelRename?.();
  };

  return (
    <div>
      <div className={`row${selected ? " is-selected" : ""}${previewPath === file.path ? " is-previewed" : ""}`} data-testid="file-row" data-path={file.path}>
        <label className="row-check">
          <input type="checkbox" checked={selected} onChange={() => toggleSelected(file.path)} aria-label={`Select ${file.name}`} disabled={sample} />
        </label>
        <span className="row-icon">
          <KindIcon kind={file.kind} />
        </span>
        {renaming ? (
          <div className="rename-inline">
            <input
              autoFocus
              value={stem}
              onChange={(e) => setStem(e.target.value)}
              onKeyDown={onRenameKey}
              aria-label="New name"
              aria-invalid={renameError ? true : undefined}
              data-testid="rename-input"
            />
            <span className="ext">{extOf(file.name)}</span>
            <button type="button" className="icon-button" onClick={() => void submitRename()} aria-label="Save name">
              <Check size={20} aria-hidden />
            </button>
            <button type="button" className="icon-button" onClick={onCancelRename} aria-label="Cancel rename">
              <X size={20} aria-hidden />
            </button>
            {renameError && <span className="field-error">{renameError}</span>}
          </div>
        ) : (
          <button type="button" className="row-body" onClick={() => setPreview(previewPath === file.path ? null : file.path)} disabled={sample}>
            <span className="row-name">
              <span className="row-name-text">{file.name}</span>
              {latest && others.length > 0 && <span className="latest">Latest</span>}
              {sample && <span className="sample">Sample</span>}
            </span>
            <span className="row-meta">
              {file.originLabel && file.downloadedAt && <span className="tone" data-tone={file.originLabel} aria-hidden />}
              {originLine(file)}
            </span>
            {file.snippet ? <span className="row-snippet" dangerouslySetInnerHTML={{ __html: snippetHtml(file.snippet) }} /> : <span className="row-where">{locationLine(file.parent, roots)}</span>}
          </button>
        )}
        <div className="row-actions">
          <button type="button" className="button button-primary" onClick={() => void run(() => ipc.open(file.path))} data-tour={tourOpen ? "open" : undefined}>
            Open
          </button>
          <button type="button" className="button" onClick={() => void run(() => ipc.reveal(file.path))}>
            <FolderOpen size={18} aria-hidden />
            Show in folder
          </button>
          <button type="button" className="icon-button" onClick={() => void run(() => ipc.copyFiles([file.path]), "Copied. Paste it into your message.")} aria-label={`Copy ${file.name}`} title="Copy the file to paste into a message">
            <Copy size={20} aria-hidden />
          </button>
          <button
            type="button"
            className={`icon-button${file.pinned ? " is-on" : ""}`}
            onClick={() => void run(async () => {
              await ipc.setPinned(file.path, !file.pinned);
              await refresh();
            }, file.pinned ? "Unpinned." : "Pinned to the top of Find.")}
            aria-label={file.pinned ? `Unpin ${file.name}` : `Pin ${file.name}`}
            aria-pressed={file.pinned}
            title={file.pinned ? "Unpin" : "Pin to the top"}
          >
            {file.pinned ? <PinOff size={20} aria-hidden /> : <Pin size={20} aria-hidden />}
          </button>
        </div>
      </div>
      {others.length > 0 && (
        <div className="copies">
          <button type="button" className="button button-quiet copies-toggle" onClick={() => setShowCopies((v) => !v)} aria-expanded={showCopies}>
            {showCopies ? "Hide copies" : copiesLabel(others.length)}
          </button>
          {showCopies && others.map((other) => <ResultRow key={other.path} file={other} roots={roots} />)}
        </div>
      )}
    </div>
  );
}

/** The snippet uses [ ] around matches (set in index.rs); render them as marks. */
export function snippetHtml(snippet: string): string {
  const escaped = snippet.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  return escaped.replace(/\[([^\]]+)\]/g, "<mark>$1</mark>");
}
