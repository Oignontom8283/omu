import { ReactFlowProvider } from '@xyflow/react';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App.tsx';
import { I18nProvider } from './i18n/index.tsx';
import { ThemeProvider } from './theme.tsx';
import './styles.css';

createRoot(document.getElementById('root') as HTMLElement).render(
  <StrictMode>
    <ThemeProvider>
      <I18nProvider>
        <ReactFlowProvider>
          <App />
        </ReactFlowProvider>
      </I18nProvider>
    </ThemeProvider>
  </StrictMode>,
);
