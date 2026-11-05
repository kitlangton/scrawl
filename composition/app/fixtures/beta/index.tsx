import * as React from 'react';
import type { StreamModule } from '@apparent/app-contracts';
function Screen() { return <><h2>Beta screen</h2><p>Synthetic Beta workflow.</p></>; }
function Settings() {
  const [value, setValue] = React.useState(() => localStorage.getItem('synthetic.beta.detail') ?? 'brief');
  return <label>Beta detail<select value={value} onChange={event => {
    localStorage.setItem('synthetic.beta.detail', event.target.value); setValue(event.target.value);
  }}><option value="brief">Brief</option><option value="full">Full</option></select></label>;
}
export default { id: 'beta', Screen, Settings } satisfies StreamModule;
