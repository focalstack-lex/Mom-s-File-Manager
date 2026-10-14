// One place that maps a file kind to its icon, so rows, previews and
// suggestions all agree, plus the Tote mark.

import { File, FileImage, FileSpreadsheet, FileText, FileType, FileVideo, Presentation } from "lucide-react";
import type { Kind } from "../lib/types";

export function KindIcon({ kind, size = 24 }: { kind: Kind; size?: number }) {
  const props = { size, strokeWidth: 1.75, "aria-hidden": true };
  switch (kind) {
    case "word":
      return <FileText {...props} />;
    case "pdf":
      return <FileType {...props} />;
    case "excel":
      return <FileSpreadsheet {...props} />;
    case "powerpoint":
      return <Presentation {...props} />;
    case "images":
      return <FileImage {...props} />;
    case "videos":
      return <FileVideo {...props} />;
    default:
      return <File {...props} />;
  }
}

/** The kind icon over the file extension, so a Word file and a PDF never look alike. */
export function KindTile({ kind, ext }: { kind: Kind; ext?: string }) {
  const label = (ext ?? "").replace(/^\./, "").slice(0, 4);
  return (
    <span className="kind-tile" aria-hidden>
      <KindIcon kind={kind} size={20} />
      {label && <span className="kind-tile-ext">{label}</span>}
    </span>
  );
}

/** Tote's mark: two filed sheets resting in a tray. Drawn, not borrowed from the nav icons. */
export function ToteMark({ size = 22 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="M7 4h7l3 3v6" />
      <path d="M7 4v9" />
      <path d="M10 9h4" />
      <path d="M3 13h5l1.5 2.5h5L16 13h5v5a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
    </svg>
  );
}
