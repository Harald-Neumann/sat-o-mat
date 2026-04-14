import { useState } from 'react';
import { LayoutDashboard, Settings, KeyRound, Check } from 'lucide-react';
import styles from './Sidebar.module.css';

export type Screen = 'dashboard' | 'settings';

interface SidebarProps {
  active: Screen;
  onNavigate: (screen: Screen) => void;
  onApiKeyChange: (key: string) => void;
}

const navItems: { screen: Screen; icon: typeof LayoutDashboard; label: string }[] = [
  { screen: 'dashboard', icon: LayoutDashboard, label: 'Dashboard' },
  { screen: 'settings', icon: Settings, label: 'Settings' },
];

export function Sidebar({ active, onNavigate, onApiKeyChange }: SidebarProps) {
  const [showKeyInput, setShowKeyInput] = useState(false);
  const [keyValue, setKeyValue] = useState(localStorage.getItem('api_key') ?? '');
  const hasKey = !!localStorage.getItem('api_key');

  function submitKey() {
    localStorage.setItem('api_key', keyValue);
    onApiKeyChange(keyValue);
    setShowKeyInput(false);
  }

  return (
    <nav className={styles.sidebar}>
      {navItems.map(({ screen, icon: Icon, label }) => (
        <button
          key={screen}
          className={`${styles.navButton} ${active === screen ? styles.active : ''}`}
          onClick={() => onNavigate(screen)}
          title={label}
        >
          <Icon size={20} />
        </button>
      ))}
      <div className={styles.spacer} />
      <div className={styles.apiKeySection}>
        {showKeyInput && (
          <div className={styles.keyPopover}>
            <input
              className={styles.keyInput}
              type="password"
              placeholder="API key"
              value={keyValue}
              onChange={(e) => setKeyValue(e.target.value)}
              onKeyDown={(e) => e.key === 'Enter' && submitKey()}
              autoFocus
            />
            <button className={styles.keySubmit} onClick={submitKey} title="Save">
              <Check size={14} />
            </button>
          </div>
        )}
        <button
          className={`${styles.navButton} ${hasKey ? styles.keySet : ''}`}
          onClick={() => setShowKeyInput(!showKeyInput)}
          title={hasKey ? 'API key set' : 'Set API key'}
        >
          <KeyRound size={20} />
        </button>
      </div>
    </nav>
  );
}
