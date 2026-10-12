// Settings > Organize: the words in a file name that point to each subject
// folder (spec 5.11). Starts from the standard DepEd list; every change is
// saved when she leaves a field. Grades need no words.

import { Plus, RotateCcw, X } from "lucide-react";
import { useEffect, useState } from "react";
import { ipc } from "../lib/ipc";
import { parseWords, wordsText } from "../lib/organize";
import type { SubjectRule } from "../lib/types";

interface Row {
  folder: string;
  words: string;
}

const toRows = (rules: SubjectRule[]): Row[] => rules.map((r) => ({ folder: r.folder, words: wordsText(r.words) }));
const toRules = (rows: Row[]): SubjectRule[] => rows.map((r) => ({ folder: r.folder.trim(), words: parseWords(r.words) })).filter((r) => r.folder && r.words.length > 0);

export function SubjectRules({ saved, onSave, onError }: { saved: SubjectRule[]; onSave(rules: SubjectRule[]): void; onError(error: unknown): void }) {
  const [rows, setRows] = useState<Row[] | null>(saved.length > 0 ? toRows(saved) : null);

  useEffect(() => {
    if (rows !== null) return;
    ipc.defaultSubjectRules().then((d) => setRows(toRows(d))).catch(onError);
  }, [rows, onError]);

  if (rows === null) return <p className="field-note">Loading the subject list...</p>;

  const commit = (next: Row[]) => {
    setRows(next);
    onSave(toRules(next));
  };
  const change = (i: number, patch: Partial<Row>) => setRows(rows.map((r, j) => (j === i ? { ...r, ...patch } : r)));

  const restoreStandard = async () => {
    try {
      const d = await ipc.defaultSubjectRules();
      setRows(toRows(d));
      // Saving nothing means "the standard list", so later improvements to it reach her too.
      onSave([]);
    } catch (error) {
      onError(error);
    }
  };

  return (
    <div className="settings-list" data-testid="subject-rules">
      {rows.map((row, i) => (
        <div key={i} className="rule-row" data-testid="subject-rule">
          <div className="field">
            <label htmlFor={`rule-folder-${i}`}>Folder</label>
            <input id={`rule-folder-${i}`} value={row.folder} onChange={(e) => change(i, { folder: e.target.value })} onBlur={() => commit(rows)} />
          </div>
          <div className="field">
            <label htmlFor={`rule-words-${i}`}>Words in the file name, separated by commas</label>
            <input id={`rule-words-${i}`} value={row.words} onChange={(e) => change(i, { words: e.target.value })} onBlur={() => commit(rows)} />
          </div>
          <button type="button" className="icon-button" onClick={() => commit(rows.filter((_, j) => j !== i))} aria-label={`Remove ${row.folder || "this subject"}`} title="Remove">
            <X size={20} aria-hidden />
          </button>
        </div>
      ))}
      <div className="shelf">
        <button type="button" className="button" onClick={() => setRows([...rows, { folder: "", words: "" }])} data-testid="add-subject">
          <Plus size={18} aria-hidden />
          Add a subject
        </button>
        <button type="button" className="button button-quiet" onClick={() => void restoreStandard()}>
          <RotateCcw size={18} aria-hidden />
          Use the standard list
        </button>
      </div>
    </div>
  );
}
