// One file on the Find screen: name, where it came from, where it is now.
// Click (or Space) previews, double-click (or Enter) opens, arrow keys move
// between files. Open, Show in folder, Copy and Pin wait quietly on the right
// and come forward on hover, focus, selection or preview; the preview panel
// carries the same actions with labels.

import { Check, Copy, FolderOpen, Pin, X } from "lucide-react";
import { useId, useState, type KeyboardEvent } from "react";
import { focusSiblingRow } from "../lib/focus";
import { ipc } from "../lib/ipc";
import { copiesLabel, extOf, locationLine, originLine, stemOf } from "../lib/format";
import type { FileRow } from "../lib/types";
import { useApp } from "../store/app";
import { useFind } from "../store/find";
import { useTour } from "../store/tour";
import { KindTile } from "./icons";

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
  const touring = useTour((s) => s.active) && tourOpen;
  const [showCopies, setShowCopies] = useState(false);
  const [stem, setStem] = useState(stemOf(file.name));
  const [renameError, setRenameError] = useState<string | null>(null);
  const metaId = useId();

  const run = async (action: () => Promise<unknown>, done?: string) => {
    if (sample) return;
    try {
      await action();
      if (done) toast(done);
    } catch (error) {
      fail(error);
    }
  };

  const open = () => void run(() => ipc.open(file.path));

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

  const onRowKey = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      focusSiblingRow(event.currentTarget, event.key === "ArrowDown" ? 1 : -1);
    } else if (event.key === "Enter") {
      event.preventDefault();
      open();
    }
  };

  const previewed = previewPath === file.path;
  const where = locationLine(file.parent, roots);

  return (
    <div>
      <div className={`row${selected ? " is-selected" : ""}${previewed ? " is-previewed" : ""}${touring ? " is-revealed" : ""}`} data-testid="file-row" data-path={file.path}>
        <label className="row-check">
          <input type="checkbox" checked={selected} onChange={() => toggleSelected(file.path)} aria-label={`Select ${file.name}`} disabled={sample} />
        </label>
        <KindTile kind={file.kind} ext={file.ext} />
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
          <button
            type="button"
            className="row-body"
            onClick={() => setPreview(previewed ? null : file.path)}
            onDoubleClick={open}
            onKeyDown={onRowKey}
            disabled={sample}
            aria-label={file.name}
            aria-describedby={metaId}
            aria-expanded={previewed}
            title={sample ? undefined : "Click to preview, double-click to open"}
          >
            <span className="row-name">
              <span className="row-name-text">{file.name}</span>
              {latest && others.length > 0 && <span className="latest">Latest</span>}
              {sample && <span className="sample">Sample</span>}
            </span>
            <span id={metaId} className="visually-hidden">
              {originLine(file)}, {where}
            </span>
            <span className="row-meta" aria-hidden>
              {file.originLabel && file.downloadedAt && <span className="tone" data-tone={file.originLabel} />}
              <span>{originLine(file)}</span>
            </span>
            {file.snippet ? <span className="row-snippet" aria-hidden dangerouslySetInnerHTML={{ __html: snippetHtml(file.snippet) }} /> : <span className="row-where" aria-hidden>{where}</span>}
          </button>
        )}
        <div className="row-actions">
          <button type="button" className="button button-subtle row-open" onClick={open} data-tour={tourOpen ? "open" : undefined}>
            Open
          </button>
          <span className="row-tools">
            <button type="button" className="icon-button row-reveal" onClick={() => void run(() => ipc.reveal(file.path))} aria-label={`Show ${file.name} in its folder`} title="Show in folder">
              <FolderOpen size={20} aria-hidden />
            </button>
            <button type="button" className="icon-button row-reveal" onClick={() => void run(() => ipc.copyFiles([file.path]), "Copied. Paste it into your message.")} aria-label={`Copy ${file.name}`} title="Copy the file to paste into a message">
              <Copy size={20} aria-hidden />
            </button>
            <button
              type="button"
              className={`icon-button${file.pinned ? " is-on" : " row-reveal"}`}
              onClick={() => void run(async () => {
                await ipc.setPinned(file.path, !file.pinned);
                await refresh();
              }, file.pinned ? "Unpinned." : "Pinned to the top of Find.")}
              aria-label={file.pinned ? `Unpin ${file.name}` : `Pin ${file.name}`}
              aria-pressed={file.pinned}
              title={file.pinned ? "Unpin" : "Pin to the top"}
            >
              <Pin size={20} aria-hidden fill={file.pinned ? "currentColor" : "none"} />
            </button>
          </span>
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

/**
 * Matches arrive wrapped in U+E000 and U+E001 (MARK_START and MARK_END in
 * index.rs), private-use characters no document holds; render them as marks
 * and everything else, brackets included, as plain text.
 */
export function snippetHtml(snippet: string): string {
  const escaped = snippet.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  return escaped.replace(/\uE000([^\uE000\uE001]*)\uE001/g, "<mark>$1</mark>").replace(/[\uE000\uE001]/g, "");
}
