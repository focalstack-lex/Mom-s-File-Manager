// One place that maps a file kind to its icon, so rows, previews and
// suggestions all agree.

import { File, FileSpreadsheet, FileText, Film, Image, Presentation } from "lucide-react";
import type { Kind } from "../lib/types";

export function KindIcon({ kind, size = 24 }: { kind: Kind; size?: number }) {
  const props = { size, strokeWidth: 1.75, "aria-hidden": true };
  switch (kind) {
    case "word":
      return <FileText {...props} />;
    case "pdf":
      return <FileText {...props} />;
    case "excel":
      return <FileSpreadsheet {...props} />;
    case "powerpoint":
      return <Presentation {...props} />;
    case "images":
      return <Image {...props} />;
    case "videos":
      return <Film {...props} />;
    default:
      return <File {...props} />;
  }
}
