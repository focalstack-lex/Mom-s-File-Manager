// Every failure reaches her as one plain sentence with what to do next.

import type { ToteError } from "./types";

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
      default:
        return error.message || "Something went wrong. Nothing was changed.";
    }
  }
  if (error instanceof Error) return error.message;
  return "Something went wrong. Nothing was changed.";
}
