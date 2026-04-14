import { useState } from 'react';
import { ChevronLeft, ChevronRight } from 'lucide-react';
import type { TaskListEntry, TaskState } from '../../api/types';
import styles from './TaskTable.module.css';

const PAGE_SIZE = 15;

const stateStyleMap: Record<TaskState, string> = {
  Active: styles.stateActive,
  PendingApproval: styles.statePendingApproval,
  Completed: styles.stateCompleted,
  Failed: styles.stateFailed,
};

const stateLabel: Record<TaskState, string> = {
  Active: 'Active',
  PendingApproval: 'Pending',
  Completed: 'Completed',
  Failed: 'Failed',
};

function formatTime(iso: string | null): string {
  if (!iso) return '\u2014';
  const d = new Date(iso);
  return d.toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  });
}

function formatDuration(start: string | null, end: string | null): string {
  if (!start || !end) return '\u2014';
  const ms = new Date(end).getTime() - new Date(start).getTime();
  if (ms < 0) return '\u2014';
  const totalSec = Math.floor(ms / 1000);
  const h = Math.floor(totalSec / 3600);
  const m = Math.floor((totalSec % 3600) / 60);
  const s = totalSec % 60;
  if (h > 0) return `${h}h ${m}m ${s}s`;
  if (m > 0) return `${m}m ${s}s`;
  return `${s}s`;
}

interface TaskTableProps {
  tasks: TaskListEntry[];
  hoveredTaskId?: string | null;
  onTaskClick?: (id: string) => void;
  onTaskHover?: (id: string | null) => void;
}

export function TaskTable({ tasks, hoveredTaskId, onTaskClick, onTaskHover }: TaskTableProps) {
  const [page, setPage] = useState(0);
  const totalPages = Math.max(1, Math.ceil(tasks.length / PAGE_SIZE));
  const start = page * PAGE_SIZE;
  const visible = tasks.slice(start, start + PAGE_SIZE);

  return (
    <div className={styles.container}>
      <div className={styles.tableScroll}>
        {tasks.length === 0 ? (
          <div className={styles.empty}>No tasks</div>
        ) : (
          <table className={styles.table}>
            <thead>
              <tr>
                <th>ID</th>
                <th>State</th>
                <th>Start</th>
                <th>End</th>
                <th>Duration</th>
              </tr>
            </thead>
            <tbody>
              {visible.map((t) => (
                <tr
                  key={t.id}
                  className={[
                    onTaskClick ? styles.clickable : '',
                    hoveredTaskId === t.id ? styles.highlighted : '',
                  ].filter(Boolean).join(' ') || undefined}
                  onClick={() => onTaskClick?.(t.id)}
                  onMouseEnter={() => onTaskHover?.(t.id)}
                  onMouseLeave={() => onTaskHover?.(null)}
                >
                  <td className={styles.mono}>{t.id}</td>
                  <td>
                    <span className={`${styles.state} ${stateStyleMap[t.state]}`}>
                      {stateLabel[t.state]}
                    </span>
                  </td>
                  <td className={styles.time}>{formatTime(t.start)}</td>
                  <td className={styles.time}>{formatTime(t.end)}</td>
                  <td className={styles.time}>{formatDuration(t.start, t.end)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
      {tasks.length > PAGE_SIZE && (
        <div className={styles.pagination}>
          <span>
            {start + 1}&ndash;{Math.min(start + PAGE_SIZE, tasks.length)} of{' '}
            {tasks.length}
          </span>
          <div className={styles.pageControls}>
            <button
              className={styles.pageButton}
              disabled={page === 0}
              onClick={() => setPage(page - 1)}
              title="Previous page"
            >
              <ChevronLeft size={16} />
            </button>
            <button
              className={styles.pageButton}
              disabled={page >= totalPages - 1}
              onClick={() => setPage(page + 1)}
              title="Next page"
            >
              <ChevronRight size={16} />
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
