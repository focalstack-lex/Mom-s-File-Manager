// One place that maps a file kind to its icon, so rows, previews and
// suggestions all agree, plus the Tote mark.

import { File, FileImage, FileSpreadsheet, FileText, FileType, FileVideo, Presentation } from "lucide-react";
import { useId } from "react";
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

/**
 * Tote's logo: a tote bag that is also an open folder of papers, in one
 * color. Same geometry as brand/tote-logo.svg; the cream parts of the
 * two-tone logo become cutouts, so the shell's green shows through them.
 */
export function ToteMark({ size = 24 }: { size?: number }) {
  // Mask and clip ids must be unique per instance and safe inside url().
  const id = useId().replace(/[^a-zA-Z0-9_-]/g, "");
  return (
    <svg width={(size * 228) / 255} height={size} viewBox="83 24 228 255" aria-hidden>
      <defs>
        <clipPath id={`${id}-inner`}>
          <rect x="94" y="119" width="196" height="158.5" />
        </clipPath>
        <mask id={`${id}-mask`} maskUnits="userSpaceOnUse" x="83" y="24" width="228" height="255">
          <path d="M146.5 104.5V77A44 44 0 0 1 234.5 77V78.5" fill="none" stroke="#fff" strokeWidth="15.5" />
          <path d="M206.5 104.5L213.5 89Q214.5 86.8 217 86.8H282.5Q285 86.8 286 89.5L288.5 104.5Z" fill="#fff" />
          <path d="M84 104H298V159.5H288V119H94V277.5H84Z" fill="#fff" />
          <g clipPath={`url(#${id}-inner)`}>
            <path d="M126.6 133H268.4L221.4 270H85.2Z" fill="#fff" />
            <path d="M133.2 140.3H258.2L213.4 270H93.6Z" fill="#000" />
            <path d="M155.2 151H280L236.9 270H72.2Z" fill="#fff" />
            <path d="M160 158.3H269.8L228.9 270H81.2Z" fill="#000" />
          </g>
          <path d="M180.3 168.3H309.2L289.8 277.5H84V257.5H94L140.5 199.8Z" fill="#fff" />
          <path d="M181 177.8L179 199H153.8Z" fill="#000" />
        </mask>
      </defs>
      <rect x="83" y="24" width="228" height="255" fill="currentColor" mask={`url(#${id}-mask)`} />
    </svg>
  );
}
