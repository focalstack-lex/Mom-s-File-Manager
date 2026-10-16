// What she typed, cut into words the way the index cuts names and contents
// (`index::words` in src-tauri/src/index.rs): lower case, split on every
// character that is not a letter or digit. "Book1.xlsx" is "book1" and
// "xlsx"; "---" is no word at all, so it is no search.

/** The words in `text`, as the index cuts them. */
export function words(text: string): string[] {
  return text.toLowerCase().split(/[^\p{L}\p{N}]+/u).filter(Boolean);
}

/** True when `text` holds at least one word: only then is typed text a search. */
export function hasWord(text: string): boolean {
  return words(text).length > 0;
}
