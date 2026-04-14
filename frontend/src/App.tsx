import { useState } from 'react';
import { Sidebar, type Screen } from './components/Sidebar/Sidebar';
import { Header } from './components/Header/Header';
import { Dashboard } from './screens/Dashboard/Dashboard';
import styles from './App.module.css';

function App() {
  const [screen, setScreen] = useState<Screen>('dashboard');
  const [apiKeyVersion, setApiKeyVersion] = useState(0);

  return (
    <div className={styles.layout}>
      <Sidebar
        active={screen}
        onNavigate={setScreen}
        onApiKeyChange={() => setApiKeyVersion((v) => v + 1)}
      />
      <div className={styles.main}>
        <Header />
        <main className={styles.content}>
          {screen === 'dashboard' && <Dashboard key={apiKeyVersion} />}
          {screen === 'settings' && (
            <div className={styles.placeholder}>Settings</div>
          )}
        </main>
      </div>
    </div>
  );
}

export default App;
