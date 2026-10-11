// Placeholder rows while a list loads, shaped like the rows that will
// replace them so nothing jumps. Screen readers hear the label once.

export function SkeletonList({ label, rows = 3, bare }: { label: string; rows?: number; bare?: boolean }) {
  return (
    <div className={bare ? undefined : "rows"} role="status" aria-label={`${label}...`}>
      {Array.from({ length: rows }, (_, i) => (
        <div key={i} className="skeleton-row" aria-hidden>
          <span className="skeleton is-tile" />
          <span className="skeleton-lines">
            <span className="skeleton" style={{ width: `${62 - i * 9}%` }} />
            <span className="skeleton" style={{ width: `${38 + i * 6}%`, height: 11 }} />
          </span>
        </div>
      ))}
    </div>
  );
}
