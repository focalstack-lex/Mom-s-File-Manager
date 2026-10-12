// The foot of a long list: how much is showing, and one button for the next page.

import { ChevronDown } from "lucide-react";
import { moreLabel } from "../lib/paging";

interface Props {
  shown: number;
  total: number;
  step: number;
  onMore(): void;
  testId?: string;
}

export function ShowMore({ shown, total, step, onMore, testId }: Props) {
  const label = moreLabel(shown, total, step);
  if (!label) return null;
  return (
    <div className="show-more" data-testid={testId}>
      <span className="show-more-count">
        Showing {shown.toLocaleString()} of {total.toLocaleString()}
      </span>
      <button type="button" className="button" onClick={onMore}>
        <ChevronDown size={18} aria-hidden />
        {label}
      </button>
    </div>
  );
}
