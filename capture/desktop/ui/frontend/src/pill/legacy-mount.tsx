import {createRoot} from 'react-dom/client';
import {RecordingPill} from '@apparent/record/ui';
import '@apparent/record/ui/style.css';

/** Keep the installed legacy window and its privileged bridge as the sole host. */
export function mountLegacyPill(element:HTMLElement|null) {
  if(!element)throw new Error('Missing recording pill root');
  createRoot(element).render(<RecordingPill host={window.apparentPill}/>);
}
