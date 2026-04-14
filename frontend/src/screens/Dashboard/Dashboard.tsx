import { useCallback, useEffect, useState } from 'react';
import moment from 'moment';
import { Plus } from 'lucide-react';
import { listTasks } from '../../api/tasks';
import type { ApiPass, TaskListEntry } from '../../api/types';
import { TaskTimeline } from '../../components/Timeline/TaskTimeline';
import { TaskModal, type TaskModalMode } from '../../components/TaskModal/TaskModal';
import { SatellitePasses } from '../../components/SatellitePasses/SatellitePasses';
import { PolarPlotDialog } from '../../components/SatellitePasses/PolarPlotDialog';
import { WidgetGrid, type Widget } from '../../components/WidgetGrid/WidgetGrid';
import styles from './Dashboard.module.css';

const defaultRange: [number, number] = [
  moment().subtract(12, 'hours').valueOf(),
  moment().add(12, 'hours').valueOf(),
];

export function Dashboard() {
  const [tasks, setTasks] = useState<TaskListEntry[]>([]);
  const [modalMode, setModalMode] = useState<TaskModalMode | null>(null);
  const [timeRange, setTimeRange] = useState<[number, number]>(defaultRange);
  const [selectedPass, setSelectedPass] = useState<{ satellite: string; pass: ApiPass } | null>(null);

  const refreshTasks = useCallback(() => {
    listTasks()
      .then(setTasks)
      .catch((err) => console.error('Failed to fetch tasks:', err));
  }, []);

  useEffect(refreshTasks, [refreshTasks]);

  const handleTaskSelect = useCallback((id: string) => {
    setModalMode({ kind: 'edit', taskId: id });
  }, []);

  const handleSaved = useCallback(() => {
    setModalMode(null);
    refreshTasks();
  }, [refreshTasks]);

  const widgets: Widget[] = [
    {
      key: 'timeline',
      title: 'Schedule',
      headerActions: (
        <button
          className={styles.newTaskButton}
          onClick={() => setModalMode({ kind: 'create' })}
          title="New task"
        >
          <Plus size={14} />
        </button>
      ),
      content: (
        <TaskTimeline
          tasks={tasks}
          onTaskSelect={handleTaskSelect}
          timeRange={timeRange}
          onTimeRangeChange={setTimeRange}
        />
      ),
      layout: { i: 'timeline', x: 0, y: 0, w: 12, h: 8, minH: 5 },
    },
    {
      key: 'passes',
      title: 'Satellite Passes',
      content: (
        <SatellitePasses
          timeRange={timeRange}
          onPassSelect={(satellite, pass) => setSelectedPass({ satellite, pass })}
        />
      ),
      layout: { i: 'passes', x: 0, y: 8, w: 12, h: 7, minH: 4 },
    },
  ];

  return (
    <div className={styles.dashboardRoot}>
      <WidgetGrid widgets={widgets} />
      {modalMode && (
        <TaskModal
          mode={modalMode}
          onClose={() => setModalMode(null)}
          onSaved={handleSaved}
        />
      )}
      {selectedPass && (
        <PolarPlotDialog
          satellite={selectedPass.satellite}
          pass={selectedPass.pass}
          onClose={() => setSelectedPass(null)}
        />
      )}
    </div>
  );
}
