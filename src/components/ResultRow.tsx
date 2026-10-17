// One file on the Find screen: name, where it came from, where it is now,
// and for a content match the words that matched as well. Click (or Space)
// previews, double-click (or Enter) opens, arrow keys move between files.
// Open, Show in folder, Copy and Pin wait quietly on the right and come
// forward on hover, focus, selection or preview; the preview panel carries
// the same actions with labels. A program, script or shortcut offers "Open
// the program", so a plain Open never runs one by surprise. Rename checks the
// name on every key with the native rules and says what is wrong under the
// field; a name that is taken is a question ("Use Name (2) instead?"), not a
// refusal. Files that only share the name sit behind their own button, apart
// from the copies.

import { Check, Copy, FolderOpen, Pin, X } from "lucide-react";
import { useId, useState, type KeyboardEvent } from "react";
import { describeError, isToteError } from "../lib/errors";
import { focusSiblingRow, focusSoon, refocusIfLost, rowTarget } from "../lib/focus";
import { ipc } from "../lib/ipc";
import { copiesLabel, extOf, isProgram, locationLine, nameProblem, numberedName, originLine, sameNameLabel, stemOf } from "../lib/format";
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
  /** Different files that share this one's name: listed apart from the copies. */
  sameName?: FileRow[];
  sample?: boolean;
  tourOpen?: boolean;
  /** Pinned rows: name and location on one line when the window is short (styles.css). */
  compact?: boolean;
  renaming?: boolean;
  onRenamed?(): void;
  onCancelRename?(): void;
}

export function ResultRow({ file, roots, latest, others = [], sameName = [], sample, tourOpen, compact, renaming, onRenamed, onCancelRename }: Props) {
  const toast = useApp((s) => s.toast);
  const fail = useApp((s) => s.fail);
  const selected = useFind((s) => s.selected.includes(file.path));
  const toggleSelected = useFind((s) => s.toggleSelected);
  const previewPath = useFind((s) => s.previewPath);
  const setPreview = useFind((s) => s.setPreview);
  const refresh = useFind((s) => s.refresh);
  const touring = useTour((s) => s.active) && tourOpen;
  const [showCopies, setShowCopies] = useState(false);
  const [showSameName, setShowSameName] = useState(false);
  const [stem, setStem] = useState(stemOf(file.name));
  const [renameError, setRenameError] = useState<string | null>(null);
  /** The name she asked for when it was taken: the field then asks whether to use "Name (2)". */
  const [clash, setClash] = useState<string | null>(null);
  const metaId = useId();
  const renameNoteId = useId();
  const ext = extOf(file.name);
  // Checked as she types, with the sentences paths::validate_name answers with.
  const typedProblem = renaming ? nameProblem(stem.trim()) : null;
  const renameMessage = typedProblem ?? renameError;

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

  const submitRename = async (pickFree = false) => {
    const wanted = stem.trim();
    // Only an exact match is no change: "Lesson Plan" to "Lesson plan" is a rename.
    if (wanted === stemOf(file.name)) {
      onCancelRename?.();
      return;
    }
    // The sentence is already under the field.
    if (nameProblem(wanted)) return;
    try {
      const renamed = await ipc.renameFile(file.path, wanted, pickFree);
      setRenameError(null);
      setClash(null);
      onRenamed?.();
      await refresh();
      // The name the file really got, "(2)" included.
      toast(`Renamed to ${renamed.split("\\").pop() ?? `${wanted}${ext}`}`);
      // The field is gone; the same file, under its new name, keeps the focus.
      focusSoon(rowTarget(renamed));
    } catch (error) {
      if (!pickFree && isToteError(error) && error.code === "alreadyExists") {
        setRenameError(null);
        setClash(wanted);
        return;
      }
      // The same plain sentence a toast would show, under the field instead.
      setRenameError(describeError(error));
    }
  };

  const onStemChange = (value: string) => {
    setStem(value);
    setRenameError(null);
    setClash(null);
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
  const program = isProgram(file.ext);
  // Latest and Sample are part of what a screen reader says first; the row's
  // description says where it came from, where it is, and why it matched.
  const name = [file.name, latest && others.length > 0 ? "Latest" : null, sample ? "Sample" : null].filter(Boolean).join(", ");
  const matched = file.snippet ? `. Inside the file: ${snippetText(file.snippet)}` : "";

  return (
    <div>
      <div className={`row${selected ? " is-selected" : ""}${previewed ? " is-previewed" : ""}${touring ? " is-revealed" : ""}${compact ? " is-compact" : ""}`} data-testid="file-row" data-path={file.path} data-tour={tourOpen ? "recent" : undefined}>
        <label className="row-check">
          <input type="checkbox" checked={selected} onChange={() => toggleSelected(file.path)} aria-label={`Select ${file.name}`} disabled={sample} />
        </label>
        <KindTile kind={file.kind} ext={file.ext} />
        {renaming ? (
          <div className="rename-inline">
            <input
              autoFocus
              value={stem}
              onChange={(e) => onStemChange(e.target.value)}
              onKeyDown={onRenameKey}
              aria-label="New name"
              aria-invalid={renameMessage ? true : undefined}
              aria-describedby={renameNoteId}
              data-testid="rename-input"
            />
            <span className="ext">{ext}</span>
            <button type="button" className="icon-button" onClick={() => void submitRename()} aria-label="Save name">
              <Check size={20} aria-hidden />
            </button>
            <button type="button" className="icon-button" onClick={onCancelRename} aria-label="Cancel rename">
              <X size={20} aria-hidden />
            </button>
            {/* Always on the page, so a screen reader announces each new sentence as it appears. */}
            <span id={renameNoteId} className={clash ? "field-note" : "field-error"} aria-live="polite" data-testid="rename-note">
              {clash ? `A file called ${clash}${ext} already exists. Use ${numberedName(clash, ext)} instead?` : renameMessage}
            </span>
            {clash && (
              <button type="button" className="button" onClick={() => void submitRename(true)} data-testid="rename-use-free">
                Use {numberedName(clash, ext)}
              </button>
            )}
          </div>
        ) : (
          <button
            type="button"
            className="row-body"
            onClick={() => setPreview(previewed ? null : file.path)}
            onDoubleClick={open}
            onKeyDown={onRowKey}
            disabled={sample}
            aria-label={name}
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
              {matched}
            </span>
            <span className="row-meta" aria-hidden>
              {file.originLabel && file.downloadedAt && <span className="tone" data-tone={file.originLabel} />}
              <span>{originLine(file)}</span>
            </span>
            {/* A content match still says where the file is; the matched words come under it. */}
            <span className="row-where" aria-hidden>
              {where}
            </span>
            {file.snippet && <span className="row-snippet" aria-hidden dangerouslySetInnerHTML={{ __html: snippetHtml(file.snippet) }} />}
          </button>
        )}
        <div className="row-actions">
          <button type="button" className="button button-subtle row-open" onClick={open} data-tour={tourOpen ? "open" : undefined} aria-label={`${program ? "Open the program" : "Open"} ${file.name}`}>
            {program ? "Open the program" : "Open"}
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
                // Unpinned from the Pinned list, the row is gone: the same file's pin in Recent, or the search box.
                refocusIfLost(`[data-testid="main"] [data-testid="file-row"][data-path="${CSS.escape(file.path)}"] [aria-pressed]`);
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
      {sameName.length > 0 && (
        <div className="copies" data-testid="same-name">
          <button type="button" className="button button-quiet copies-toggle" onClick={() => setShowSameName((v) => !v)} aria-expanded={showSameName} data-testid="same-name-toggle">
            {showSameName ? "Hide the files with the same name" : sameNameLabel(sameName.length)}
          </button>
          {showSameName && sameName.map((other) => <ResultRow key={other.path} file={other} roots={roots} />)}
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

/** The snippet in plain words for a screen reader: the private-use marks dropped, the text kept. */
const SNIPPET_MARKS = new RegExp(`[${String.fromCharCode(0xe000, 0xe001)}]`, "g");

export function snippetText(snippet: string): string {
  return snippet.replace(SNIPPET_MARKS, "");
}
