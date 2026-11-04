import { useEffect, useState, type ReactNode } from 'react';
import { formatBuildIdentity, UNKNOWN_BUILD_IDENTITY } from './label';
import './build-identity.css';

declare global {
  interface Window {
    apparentBuild?: { read: () => Promise<string> };
  }
}

/** A passive indication of the build serving this renderer. */
export function BuildIdentity() {
  const [identity, setIdentity] = useState(UNKNOWN_BUILD_IDENTITY);
  useEffect(() => {
    let active = true;
    const read = window.apparentBuild?.read;
    if (!read) return;
    void read.call(window.apparentBuild).then(label => {
      if (active) setIdentity(formatBuildIdentity(label));
    }).catch(() => {
      if (active) setIdentity(UNKNOWN_BUILD_IDENTITY);
    });
    return () => { active = false; };
  }, []);
  return <span className="apparent-build-identity" aria-label={`App build ${identity}`}>{identity}</span>;
}

/** Reserve a window edge so the identity never paints over scrolling controls. */
export function BuildIdentityFrame({children}: {children: ReactNode}) {
  return <div className="apparent-window-frame">
    <div className="apparent-window-content">{children}</div>
    <footer className="apparent-window-build"><BuildIdentity /></footer>
  </div>;
}
