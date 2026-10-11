// Filter chips (spec 5.3). Date chips are always visible; type and source
// chips stay behind "More filters" so the screen is not crowded.

import { ChevronDown, ChevronRight } from "lucide-react";
import { availableChips } from "../lib/dates";
import type { Kind, Settings } from "../lib/types";
import { useFind } from "../store/find";

const KINDS: { id: Kind; label: string }[] = [
  { id: "word", label: "Word" },
  { id: "pdf", label: "PDF" },
  { id: "excel", label: "Excel" },
  { id: "powerpoint", label: "PowerPoint" },
  { id: "images", label: "Images" },
];

const SOURCES = ["Messenger", "Gmail", "Google Drive", "DepEd", "Other"];

export function Chips({ settings }: { settings: Settings }) {
  const dateChip = useFind((s) => s.dateChip);
  const setDateChip = useFind((s) => s.setDateChip);
  const kinds = useFind((s) => s.kinds);
  const toggleKind = useFind((s) => s.toggleKind);
  const sources = useFind((s) => s.sources);
  const toggleSource = useFind((s) => s.toggleSource);
  const more = useFind((s) => s.moreFilters);
  const setMore = useFind((s) => s.setMoreFilters);
  const extra = kinds.length + sources.length;

  return (
    <div className="chips-more" data-testid="chips">
      <div className="chips" role="group" aria-label="When">
        {availableChips(settings).map((chip) => (
          <button key={chip.id} type="button" className="chip" aria-pressed={dateChip === chip.id} onClick={() => setDateChip(chip.id)}>
            {chip.label}
          </button>
        ))}
        <button type="button" className="chip" onClick={() => setMore(!more)} aria-expanded={more} data-testid="more-filters">
          {more ? <ChevronDown size={18} aria-hidden /> : <ChevronRight size={18} aria-hidden />}
          {extra > 0 ? `More filters (${extra})` : "More filters"}
        </button>
      </div>
      {more && (
        <>
          <div className="chips" role="group" aria-label="File type">
            <span className="chip-group-label">Type</span>
            {KINDS.map((kind) => (
              <button key={kind.id} type="button" className="chip" aria-pressed={kinds.includes(kind.id)} onClick={() => toggleKind(kind.id)}>
                {kind.label}
              </button>
            ))}
          </div>
          <div className="chips" role="group" aria-label="Where it came from">
            <span className="chip-group-label">From</span>
            {SOURCES.map((source) => (
              <button key={source} type="button" className="chip" aria-pressed={sources.includes(source)} onClick={() => toggleSource(source)}>
                {source}
              </button>
            ))}
          </div>
        </>
      )}
    </div>
  );
}
