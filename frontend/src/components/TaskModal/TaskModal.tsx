import { useState, useEffect, useCallback, useRef } from 'react';
import { X } from 'lucide-react';
import { getTask, putTask, deleteTask } from '../../api/tasks';
import styles from './TaskModal.module.css';

type Mode = { kind: 'edit'; taskId: string } | { kind: 'create' };

interface TaskModalProps {
  mode: Mode;
  onClose: () => void;
  onSaved: () => void;
}

export type { Mode as TaskModalMode };

export function TaskModal({ mode, onClose, onSaved }: TaskModalProps) {
  const [taskId, setTaskId] = useState(mode.kind === 'edit' ? mode.taskId : '');
  const [yaml, setYaml] = useState(mode.kind === 'create' ? '' : null as string | null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const editorRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    if (mode.kind !== 'edit') return;
    let cancelled = false;
    getTask(mode.taskId)
      .then((text) => { if (!cancelled) setYaml(text); })
      .catch((err) => { if (!cancelled) setError(String(err)); });
    return () => { cancelled = true; };
  }, [mode]);

  const handleBackdropClick = useCallback(
    (e: React.MouseEvent) => {
      if (e.target === e.currentTarget) onClose();
    },
    [onClose],
  );

  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', handleKey);
    return () => window.removeEventListener('keydown', handleKey);
  }, [onClose]);

  const handleTab = useCallback((e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key !== 'Tab') return;
    e.preventDefault();
    const ta = e.currentTarget;
    const start = ta.selectionStart;
    const end = ta.selectionEnd;

    if (!e.shiftKey) {
      // Insert two spaces at cursor
      const before = ta.value.slice(0, start);
      const after = ta.value.slice(end);
      setYaml(before + '  ' + after);
      requestAnimationFrame(() => {
        ta.selectionStart = ta.selectionEnd = start + 2;
      });
    } else {
      // Dedent: remove up to 2 leading spaces from current line
      const lineStart = ta.value.lastIndexOf('\n', start - 1) + 1;
      const line = ta.value.slice(lineStart);
      const spaces = line.match(/^ {1,2}/)?.[0].length ?? 0;
      if (spaces > 0) {
        const updated = ta.value.slice(0, lineStart) + ta.value.slice(lineStart + spaces);
        setYaml(updated);
        requestAnimationFrame(() => {
          ta.selectionStart = ta.selectionEnd = Math.max(lineStart, start - spaces);
        });
      }
    }
  }, []);

  const handleSave = useCallback(async () => {
    const id = taskId.trim();
    if (!id) { setError('Task ID is required'); return; }
    if (!yaml?.trim()) { setError('YAML body is required'); return; }
    setSaving(true);
    setError(null);
    try {
      await putTask(id, yaml);
      onSaved();
    } catch (err) {
      setError(String(err));
    } finally {
      setSaving(false);
    }
  }, [taskId, yaml, onSaved]);

  const handleDelete = useCallback(async () => {
    if (mode.kind !== 'edit') return;
    if (!confirm(`Delete task "${mode.taskId}"?`)) return;
    setSaving(true);
    setError(null);
    try {
      await deleteTask(mode.taskId);
      onSaved();
    } catch (err) {
      setError(String(err));
      setSaving(false);
    }
  }, [mode, onSaved]);

  const isCreate = mode.kind === 'create';
  const loading = !isCreate && yaml === null && !error;

  return (
    <div className={styles.backdrop} onClick={handleBackdropClick}>
      <div className={styles.modal}>
        <div className={styles.header}>
          {isCreate ? (
            <input
              className={styles.titleInput}
              value={taskId}
              onChange={(e) => setTaskId(e.target.value)}
              placeholder="Task ID"
              autoFocus
              spellCheck={false}
            />
          ) : (
            <span className={styles.title}>{taskId}</span>
          )}
          <button className={styles.closeButton} onClick={onClose} title="Close">
            <X size={16} />
          </button>
        </div>
        <div className={styles.body}>
          {error && <div className={styles.error}>{error}</div>}
          {loading ? (
            <div className={styles.loading}>Loading…</div>
          ) : (
            <textarea
              ref={editorRef}
              className={styles.editor}
              value={yaml ?? ''}
              onChange={(e) => setYaml(e.target.value)}
              onKeyDown={handleTab}
              spellCheck={false}
              wrap="off"
              autoFocus={!isCreate}
            />
          )}
        </div>
        <div className={styles.footer}>
          {!isCreate && (
            <button
              className={`${styles.button} ${styles.buttonDanger}`}
              onClick={handleDelete}
              disabled={saving || loading}
            >
              Delete
            </button>
          )}
          <div className={styles.footerSpacer} />
          <button
            className={`${styles.button} ${styles.buttonPrimary}`}
            onClick={handleSave}
            disabled={saving || loading}
          >
            {saving ? 'Saving…' : isCreate ? 'Create' : 'Update'}
          </button>
        </div>
      </div>
    </div>
  );
}
