import { useState, useCallback, useRef } from 'react';
import Timeline, {
  TimelineMarkers,
  TodayMarker,
  type TimelineGroupBase,
  type TimelineItemBase,
} from 'react-calendar-timeline';
import 'react-calendar-timeline/style.css';
import moment from 'moment';
import type { TaskListEntry, TaskState } from '../../api/types';
import { TaskTable } from '../TaskTable/TaskTable';
import styles from './TaskTimeline.module.css';

const groups: TimelineGroupBase[] = [
  { id: 'Active', title: 'Active' },
  { id: 'PendingApproval', title: 'Pending' },
  { id: 'Completed', title: 'Completed' },
  { id: 'Failed', title: 'Failed' },
];

const stateStyleMap: Record<TaskState, string> = {
  Active: styles.itemActive,
  PendingApproval: styles.itemPendingApproval,
  Completed: styles.itemCompleted,
  Failed: styles.itemFailed,
};

interface TaskTimelineProps {
  tasks: TaskListEntry[];
  onTaskSelect?: (id: string) => void;
  timeRange?: [number, number];
  onTimeRangeChange?: (range: [number, number]) => void;
}

function isVisibleInRange(
  task: TaskListEntry,
  rangeStart: number,
  rangeEnd: number,
): boolean {
  if (!task.start || !task.end) return false;
  const s = new Date(task.start).getTime();
  const e = new Date(task.end).getTime();
  return s < rangeEnd && e > rangeStart;
}

export function TaskTimeline({ tasks, onTaskSelect, timeRange, onTimeRangeChange }: TaskTimelineProps) {
  const now = moment();
  const defaultStart = now.clone().subtract(12, 'hours').valueOf();
  const defaultEnd = now.clone().add(12, 'hours').valueOf();

  const [internalRange, setInternalRange] = useState<[number, number]>([
    defaultStart,
    defaultEnd,
  ]);
  const tableRange = timeRange ?? internalRange;
  const debounceRef = useRef<ReturnType<typeof setTimeout>>(undefined);
  const [hoveredTaskId, setHoveredTaskId] = useState<string | null>(null);

  const items: TimelineItemBase<number>[] = tasks
    .filter((t) => t.start && t.end)
    .map((t) => ({
      id: t.id,
      group: t.state,
      title: t.id,
      start_time: new Date(t.start!).getTime(),
      end_time: new Date(t.end!).getTime(),
      className: [
        styles.item,
        stateStyleMap[t.state],
        hoveredTaskId === t.id ? styles.itemHighlighted : '',
      ].join(' '),
    }));

  const handleTimeChange = useCallback(
    (
      start: number,
      end: number,
      updateScrollCanvas: (start: number, end: number) => void,
    ) => {
      updateScrollCanvas(start, end);
      clearTimeout(debounceRef.current);
      debounceRef.current = setTimeout(() => {
        const range: [number, number] = [start, end];
        setInternalRange(range);
        onTimeRangeChange?.(range);
      }, 200);
    },
    [onTimeRangeChange],
  );

  const handleItemSelect = useCallback(
    (itemId: string) => {
      onTaskSelect?.(String(itemId));
    },
    [onTaskSelect],
  );

  const visibleTasks = tasks.filter((t) =>
    isVisibleInRange(t, tableRange[0], tableRange[1]),
  );

  return (
    <div className={styles.timelineWrapper}>
      <div className={styles.timelineSection}>
        <Timeline
          groups={groups}
          items={items}
          defaultTimeStart={defaultStart}
          defaultTimeEnd={defaultEnd}
          onTimeChange={handleTimeChange}
          onItemSelect={handleItemSelect}
          sidebarWidth={120}
          lineHeight={36}
          itemHeightRatio={0.75}
          canMove={false}
          canResize={false}
          canChangeGroup={false}
          itemRenderer={({ item, itemContext, getItemProps }) => {
            const props = getItemProps({});
            return (
              <div
                {...props}
                onMouseEnter={(e) => {
                  setHoveredTaskId(String(item.id));
                  e.stopPropagation();
                }}
                onMouseLeave={() => setHoveredTaskId(null)}
              >
                <div
                  className="rct-item-content"
                  style={{ maxHeight: `${itemContext.dimensions.height}px` }}
                >
                  {itemContext.title}
                </div>
              </div>
            );
          }}
        >
          <TimelineMarkers>
            <TodayMarker interval={10000}>
              {({ styles: markerStyles }) => (
                <div className={styles.nowMarker} style={markerStyles} />
              )}
            </TodayMarker>
          </TimelineMarkers>
        </Timeline>
      </div>
      <div className={styles.tableSection}>
        <TaskTable
          tasks={visibleTasks}
          hoveredTaskId={hoveredTaskId}
          onTaskClick={(id) => onTaskSelect?.(id)}
          onTaskHover={setHoveredTaskId}
        />
      </div>
    </div>
  );
}
