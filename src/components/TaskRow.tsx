// One task: a checkbox, the title and one quiet line. Clicking the title
// opens the details in place: date, repeat, attached files, Delete.

import { ChevronRight, FolderOpen, Paperclip, Trash, X } from "lucide-react";
import { useCallback, useId, useState } from "react";
import { locationLine } from "../lib/format";
import { ipc } from "../lib/ipc";
import { groupOf, hasQuarters, taskMeta } from "../lib/todo";
import type { AppInfo, Repeat, Task, TaskFile } from "../lib/types";
import { useApp } from "../store/app";
import { useTodo } from "../store/todo";
import { AttachDialog } from "./AttachDialog";
import { KindTile } from "./icons";

export function TaskRow({ task, info, now }: { task: Task; info: AppInfo; now: Date }) {
  const expanded = useTodo((s) => s.openId === task.id);
  const setOpen = useTodo((s) => s.setOpen);
  const complete = useTodo((s) => s.complete);
  const reopen = useTodo((s) => s.reopen);
  const detailsId = useId();
  const done = task.doneAt !== null;
  const meta = taskMeta(task, now);
  const overdue = !done && groupOf(task.due, now) === "Overdue";

  return (
    <div className={`task${expanded ? " is-open" : ""}${done ? " is-done" : ""}`} data-testid="task-row">
      <div className="task-line">
        <label className="row-check">
          <input type="checkbox" checked={done} onChange={() => void (done ? reopen(task.id) : complete(task))} aria-label={done ? `Put ${task.title} back on the list` : `Mark ${task.title} as done`} data-testid="task-check" />
        </label>
        <button type="button" className="task-body" onClick={() => setOpen(expanded ? null : task.id)} aria-expanded={expanded} aria-controls={detailsId} data-testid="task-open">
          <span className="task-text">
            <span className="task-title">{task.title}</span>
            {meta && <span className={`task-meta${overdue ? " is-overdue" : ""}`}>{meta}</span>}
          </span>
          <ChevronRight className="task-chevron" size={20} aria-hidden />
        </button>
      </div>
      {expanded && <TaskDetails id={detailsId} task={task} info={info} />}
    </div>
  );
}

function TaskDetails({ id, task, info }: { id: string; task: Task; info: AppInfo }) {
  const update = useTodo((s) => s.update);
  const remove = useTodo((s) => s.remove);
  const [title, setTitle] = useState(task.title);
  const [attaching, setAttaching] = useState(false);
  const closeAttach = useCallback(() => setAttaching(false), []);
  const quarters = hasQuarters(info.settings);

  const saveTitle = () => {
    const next = title.trim();
    if (next === "") setTitle(task.title);
    else if (next !== task.title) void update(task, { title: next });
  };

  return (
    <div className="task-details" id={id} data-testid="task-details">
      <div className="field">
        <label htmlFor={`${id}-title`}>Task</label>
        <input
          id={`${id}-title`}
          value={title}
          maxLength={200}
          onChange={(e) => setTitle(e.target.value)}
          onBlur={saveTitle}
          onKeyDown={(e) => {
            if (e.key === "Enter") e.currentTarget.blur();
            if (e.key === "Escape") setTitle(task.title);
          }}
          data-testid="task-title"
        />
      </div>
      <div className="task-fields">
        <div className="field">
          <label htmlFor={`${id}-due`}>Due</label>
          <input id={`${id}-due`} type="date" value={task.due ?? ""} onChange={(e) => void update(task, { due: e.target.value === "" ? null : e.target.value })} data-testid="task-due" />
        </div>
        <div className="field">
          <label htmlFor={`${id}-repeat`}>Repeat</label>
          <select id={`${id}-repeat`} value={task.repeat} disabled={task.due === null} onChange={(e) => void update(task, { repeat: e.target.value as Repeat })} data-testid="task-repeat">
            <option value="never">Never</option>
            <option value="week">Every week</option>
            <option value="month">Every month</option>
            <option value="quarter" disabled={!quarters}>
              Every quarter
            </option>
          </select>
        </div>
        {task.due !== null && (
          <button type="button" className="button button-subtle" onClick={() => void update(task, { due: null })} data-testid="task-clear-due">
            No due date
          </button>
        )}
      </div>
      {task.due === null ? <p className="field-note">Pick a due date to make it repeat.</p> : !quarters && <p className="field-note">Every quarter needs the quarter dates in Settings.</p>}
      {task.files.length > 0 && (
        <div className="task-files">
          {task.files.map((file) => (
            <AttachedFile key={file.path} taskId={task.id} file={file} roots={info.roots} />
          ))}
        </div>
      )}
      <div className="task-actions">
        <button type="button" className="button" onClick={() => setAttaching(true)} data-testid="task-attach">
          <Paperclip size={18} aria-hidden />
          Attach a file
        </button>
        <button type="button" className="button button-subtle" onClick={() => void remove(task)} data-testid="task-delete">
          <Trash size={18} aria-hidden />
          Delete task
        </button>
      </div>
      {attaching && <AttachDialog taskId={task.id} roots={info.roots} onClose={closeAttach} />}
    </div>
  );
}

function AttachedFile({ taskId, file, roots }: { taskId: number; file: TaskFile; roots: string[] }) {
  const detach = useTodo((s) => s.detach);
  const fail = useApp((s) => s.fail);
  const parent = file.path.slice(0, file.path.lastIndexOf("\\"));
  return (
    <div className={`task-file${file.missing ? " is-missing" : ""}`} data-testid="task-file">
      <KindTile kind={file.kind} ext={file.ext} />
      <span className="task-file-text">
        <span className="task-file-name" title={file.path}>
          {file.name}
        </span>
        <span className="task-file-where">{file.missing ? "Can't find this file. It may have been moved or deleted." : locationLine(parent, roots)}</span>
      </span>
      <span className="task-file-actions">
        {!file.missing && (
          <>
            <button type="button" className="button button-subtle" onClick={() => void ipc.open(file.path).catch(fail)} data-testid="task-file-open">
              Open
            </button>
            <button type="button" className="icon-button" onClick={() => void ipc.reveal(file.path).catch(fail)} aria-label={`Show ${file.name} in its folder`} title="Show in folder">
              <FolderOpen size={20} aria-hidden />
            </button>
          </>
        )}
        <button type="button" className="icon-button" onClick={() => void detach(taskId, file.path)} aria-label={`Remove ${file.name} from this task`} title="Remove from this task" data-testid="task-file-remove">
          <X size={20} aria-hidden />
        </button>
      </span>
    </div>
  );
}
