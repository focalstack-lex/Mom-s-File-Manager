// Every failure reaches her as one plain sentence with what to do next.

import type { OpRow, ToteError } from "./types";

export function isToteError(value: unknown): value is ToteError {
  return typeof value === "object" && value !== null && "code" in value && "message" in value;
}

export function describeError(error: unknown): string {
  if (isToteError(error)) {
    switch (error.code) {
      case "notFound":
        return "That file is no longer where it was. Try searching for it.";
      case "permissionDenied":
        return "Windows did not allow that. The file may be open in another program.";
      case "alreadyExists":
        return error.message || "Something with that name is already there.";
      case "invalidName":
        return error.message || "That name will not work. Try another.";
      case "outsideRoots":
        return "Tote only works with files in Desktop, Documents, Downloads and OneDrive.";
      case "db":
        return "Tote could not save that. Try again in a moment.";
      case "network":
        return "Tote could not reach the internet to update. It will try again later.";
      case "inUse":
        return error.message || "That file is open in another program. Close it there, then try again.";
      case "notInRecycleBin":
        return error.message || "That file is not in the Recycle Bin any more. Open the Recycle Bin to check.";
      case "noRecycleBin":
        return error.message || "Files on a flash drive or a network folder cannot go to the Recycle Bin. Delete it in File Explorer if you are sure.";
      case "updateSignature":
        return error.message || "The update did not pass Tote's safety check, so it was not installed.";
      case "updateInstall":
        return error.message || "The update could not be installed. Try again later.";
      default:
        return error.message || "Something went wrong. Nothing was changed.";
    }
  }
  if (error instanceof Error) return error.message;
  return "Something went wrong. Nothing was changed.";
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
  return `${lead}${name}: ${failed[0].message ?? "Windows could not move it to the Recycle Bin."}`;
}
