export interface TextViolation {
  line: number;
  column: number;
  kind: "dash" | "emoji";
  sample: string;
}

export function scanText(text: string): TextViolation[];
export function isScannable(path: string): boolean;
