// Quick preview (spec 5.3): the words inside a document, the image itself,
// or plain details, beside the list.

import { convertFileSrc } from "@tauri-apps/api/core";
import { X } from "lucide-react";
import { useEffect, useState } from "react";
import { fileSize, locationLine, relativeTime } from "../lib/format";
import { ipc } from "../lib/ipc";
import { describeError } from "../lib/errors";
import type { Preview } from "../lib/types";
import { useFind } from "../store/find";

export function PreviewPanel({ roots }: { roots: string[] }) {
  const path = useFind((s) => s.previewPath);
  const setPreview = useFind((s) => s.setPreview);
  const [preview, setPreviewData] = useState<Preview | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!path) return;
    let live = true;
    setPreviewData(null);
    setError(null);
    ipc
      .preview(path)
      .then((p) => live && setPreviewData(p))
      .catch((e) => live && setError(describeError(e)));
    return () => {
      live = false;
    };
  }, [path]);

  if (!path) return null;
  const imageSrc = preview?.kind === "image" && !window.__deskMock ? convertFileSrc(path) : null;

  return (
    <aside className="card preview" aria-label="Preview" data-testid="preview">
      <div className="preview-head">
        <h2 className="preview-name">{preview?.name ?? path.split("\\").pop()}</h2>
        <button type="button" className="icon-button" onClick={() => setPreview(null)} aria-label="Close preview">
          <X size={20} aria-hidden />
        </button>
      </div>
      {error && <p className="field-error">{error}</p>}
      {preview && (
        <>
          <dl className="preview-details">
            <dt>Where</dt>
            <dd>{locationLine(preview.parent, roots)}</dd>
            <dt>Edited</dt>
            <dd>{relativeTime(preview.modified)}</dd>
            <dt>Size</dt>
            <dd>{fileSize(preview.size)}</dd>
          </dl>
          {preview.kind === "text" && preview.text && <div className="preview-text">{preview.text}</div>}
          {imageSrc && <img className="preview-image" src={imageSrc} alt={preview.name} />}
          {preview.kind === "image" && !imageSrc && <p className="field-note">Image preview is shown in the app.</p>}
          {preview.kind === "details" && <p className="field-note">Desk cannot read inside this kind of file, but it can still find it by name.</p>}
        </>
      )}
      <div className="preview-actions">
        <button type="button" className="button button-primary" onClick={() => void ipc.open(path)}>
          Open
        </button>
        <button type="button" className="button" onClick={() => void ipc.reveal(path)}>
          Show in folder
        </button>
      </div>
    </aside>
  );
}
