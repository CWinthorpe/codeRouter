import ReactDOM from 'react-dom/client';
import App from './App';
import './index.css';

/**
 * Application entry point. Creates the React 18 root and renders the
 * top-level <App /> component into the #root DOM element.
 */
const root = document.getElementById('root');
if (!root) {
  throw new Error('Root element not found');
}

ReactDOM.createRoot(root).render(
  <App />
);
