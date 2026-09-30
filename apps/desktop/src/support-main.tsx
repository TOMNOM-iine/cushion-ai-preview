import { createRoot } from 'react-dom/client';
import EmbeddedChat from './EmbeddedChat';
import './embed.css';

createRoot(document.getElementById('root')!).render(<EmbeddedChat supportRecord />);
