import { useEffect, useState } from 'react';
import { getStation } from '../../api/station';
import styles from './Header.module.css';

function formatRfc3339(date: Date): string {
  return date.toISOString().replace(/\.\d{3}Z$/, 'Z');
}

export function Header() {
  const [stationName, setStationName] = useState<string>('');
  const [now, setNow] = useState(() => new Date());

  useEffect(() => {
    getStation()
      .then((info) => setStationName(info.name))
      .catch(() => setStationName('—'));
  }, []);

  useEffect(() => {
    const id = setInterval(() => setNow(new Date()), 1000);
    return () => clearInterval(id);
  }, []);

  return (
    <header className={styles.header}>
      <span className={styles.station}>{stationName}</span>
      <span className={styles.clock}>{formatRfc3339(now)}</span>
    </header>
  );
}
