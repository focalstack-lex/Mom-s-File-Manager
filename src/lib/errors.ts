// Every failure reaches her as one plain sentence with what to do next. The
// native side words its own failures; this file keeps raw program text off
// the screen and fills in a sentence when none came.

import type { OpRow, ToteError } from "./types";

export function isToteError(value: unknown): value is ToteError {
  return typeof value === "object" && value !== null && "code" in value && "message" in value;
}

/** One fixed sentence per code, used when the native side sent none, or sent program text. */
export const CODE_SENTENCES: Record<ToteError["code"], string> = {
  notFound: "That file is no longer where it was. Try searching for it.",
  permissionDenied: "Windows did not allow that. Close the file in other programs, then try again.",
  alreadyExists: "Something with that name is already there. Try another name.",
  invalidPath: "Tote cannot use that file's location. Move it into one of your folders first.",
  invalidName: "That name will not work. Try another.",
  outsideRoots: "Tote only changes files in the folders it looks in. Check Settings.",
  io: "Windows could not finish that. Try again in a moment.",
  db: "Tote could not read its list right now. Try again in a moment.",
  validation: "That cannot be done right now. Nothing was changed.",
  network: "Tote could not reach the internet to update. It will try again later.",
  unsupported: "Tote cannot do that with this file.",
  inUse: "That file is open in another program. Close it there, then try again.",
  notInRecycleBin: "That file is not in the Recycle Bin any more. Open the Recycle Bin to check.",
  noRecycleBin: "Files on a flash drive or a network folder cannot go to the Recycle Bin. Delete it in File Explorer if you are sure.",
  updateSignature: "The update did not pass Tote's safety check, so it was not installed.",
  updateInstall: "The update could not be installed. Try again later.",
};

const GENERIC = "Something went wrong. Nothing was changed.";

/** Program text that must never reach her: OS error numbers, library prefixes, Rust debug dumps. */
const RAW_TEXT = /os error|Error during|[{}`]|::|panicked|unwrap|\bErr\(|\bError\b\s*:/;

/** True when a message reads as a plain sentence rather than program text. */
export function isPlain(message: string | null | undefined): message is string {
  return typeof message === "string" && message.trim() !== "" && !RAW_TEXT.test(message);
}

/** The message when it is plain, else the fallback sentence. */
export function plainMessage(message: string | null | undefined, fallback: string): string {
  return isPlain(message) ? message : fallback;
}

export function describeError(error: unknown): string {
  if (isToteError(error)) return plainMessage(error.message, CODE_SENTENCES[error.code] ?? GENERIC);
  // A renderer-side Error carries program text; it goes to the console (fail logs it), not to her.
  return GENERIC;
}

/**
 * After a Recycle Bin action, the sentence for what did not happen: the first
 * file that stayed, with the reason the native side gave, after a count of
 * what did move. Null when every file went to the Recycle Bin.
 */
export function recycleFailureText(ops: OpRow[]): string | null {
  const failed = ops.filter((o) => o.status === "failed");
  if (failed.length === 0) return null;
  const done = ops.length - failed.length;
  const name = failed[0].src.split(/[\\/]/).pop() || "That file";
  const lead = done > 0 ? `Moved ${done} of ${ops.length} to the Recycle Bin. ` : "";
  return `${lead}${name}: ${plainMessage(failed[0].message, "Windows could not move it to the Recycle Bin.")}`;
}

/** One file an Undo could not put back, with the reason when the native side gave one. */
export interface UndoFailure {
  name: string;
  reason: string | null;
}

const UNDO_STAYED = "It could not be moved back. It may be open in another program; close it there, then undo again from Activity.";

/**
 * What an Undo should say once every part of it has been tried: "Put back
 * where it was.", or "Put back 5 of 6. Grade 7 Quiz.docx: That file is open
 * in another program. Close it there, then try again." The same words serve
 * Find, Clean up, Organize and Activity, so she reads one sentence shape.
 */
export function undoText(undone: number, failed: UndoFailure[]): string {
  if (failed.length === 0) return undone > 1 ? "Everything is back where it was." : "Put back where it was.";
  const total = undone + failed.length;
  const first = failed[0];
  const lead = undone > 0 ? `Put back ${undone} of ${total}. ` : total > 1 ? `None of the ${total} could be put back. ` : "";
  return `${lead}${first.name}: ${plainMessage(first.reason, UNDO_STAYED)}`;
}
