// One plain line with a Try again button, shown where a list would be when
// its first load failed, so a screen never sits on a loading state for good.

import { CircleAlert } from "lucide-react";

export function LoadFailed({ text, onRetry, testId, quiet }: { text: string; onRetry(): void; testId: string; /** A status line, not an alert: for a search that failed while the last answer stays. */ quiet?: boolean }) {
  return (
    <p className="notice" role={quiet ? "status" : "alert"} data-testid={testId}>
      <CircleAlert size={18} aria-hidden />
      <span>{text}</span>
      <button type="button" className="button button-quiet" onClick={onRetry} data-testid={`${testId}-retry`}>
        Try again
      </button>
    </p>
  );
}
