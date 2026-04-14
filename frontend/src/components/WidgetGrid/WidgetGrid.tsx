import {
  Responsive,
  useContainerWidth,
  type LayoutItem,
} from 'react-grid-layout';
import 'react-grid-layout/css/styles.css';
import styles from './WidgetGrid.module.css';

export interface Widget {
  key: string;
  title: string;
  content: React.ReactNode;
  headerActions?: React.ReactNode;
  layout: LayoutItem;
}

interface WidgetGridProps {
  widgets: Widget[];
  rowHeight?: number;
}

export function WidgetGrid({ widgets, rowHeight = 60 }: WidgetGridProps) {
  const { width, containerRef } = useContainerWidth();

  const layouts = {
    lg: widgets.map((w) => w.layout),
  };

  return (
    <div ref={containerRef} className={styles.grid}>
      {width > 0 && (
        <Responsive
          width={width}
          layouts={layouts}
          breakpoints={{ lg: 0 }}
          cols={{ lg: 12 }}
          rowHeight={rowHeight}
          dragConfig={{ handle: `.${styles.widgetHeader}` }}
          margin={[8, 8] as const}
          containerPadding={[8, 8] as const}
        >
          {widgets.map((w) => (
            <div key={w.key} className={styles.widget}>
              <div className={styles.widgetHeader}>
                <span>{w.title}</span>
                {w.headerActions && (
                  <div className={styles.widgetActions}>{w.headerActions}</div>
                )}
              </div>
              <div className={styles.widgetContent}>{w.content}</div>
            </div>
          ))}
        </Responsive>
      )}
    </div>
  );
}
