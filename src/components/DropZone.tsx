// What the window shows while she drags files from File Explorer over it:
// where to let go, and what will happen. The drop itself is filed by the
// native side exactly like Move to Tote (lib.rs, filing.rs), and the usual
// "Filed ..." toast answers it.

import { FolderInput } from "lucide-react";
import { useEffect, useState } from "react";
import { onFileDrag } from "../lib/ipc";

export function DropZone() {
  const [over, setOver] = useState(false);

  useEffect(() => {
    const unlisten = onFileDrag(setOver);
    return () => {
      unlisten.then((fn) => fn()).catch(() => undefined);
    };
  }, []);

  if (!over) return null;
  return (
    <div className="drop-zone" role="status" data-testid="drop-zone">
      <div className="drop-card">
        <span className="drop-mark" aria-hidden>
          <FolderInput size={28} />
        </span>
        <h2 className="drop-title">Drop to file it</h2>
        <p className="drop-text">Each file goes to its sorted folder, like Move to Tote.</p>
      </div>
    </div>
  );
}
