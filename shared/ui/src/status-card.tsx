import type { ReactNode } from 'react';
import { cn } from './utils';
export type StatusTone = 'neutral' | 'success' | 'progress' | 'warning' | 'failure';
const paths: Record<Exclude<StatusTone, 'neutral'>, ReactNode> = {
  success: <><circle cx="12" cy="12" r="9" /><path d="m7.5 12 3 3 6-6" /></>,
  progress: <><circle cx="12" cy="12" r="9" /><path d="M12 6v6l4 2" /></>,
  warning: <><path d="M12 3 2 21h20L12 3Z" /><path d="M12 9v5m0 3v.1" /></>,
  failure: <><circle cx="12" cy="12" r="9" /><path d="m9 9 6 6m0-6-6 6" /></>,
};
export function StatusCard({ tone, headline, compact = false, artwork, children }: {
  tone: StatusTone; headline: string; compact?: boolean; artwork?: ReactNode; children: ReactNode;
}) {
  return <section className={cn('apparent-status-card', compact && 'compact')} data-tone={tone}>
    <div className="status-content">
      <h2><span className="status-heading" role={tone === 'failure' ? 'alert' : 'status'} aria-live="polite">
        {tone !== 'neutral' && <span className="status-icon" aria-hidden="true"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">{paths[tone]}</svg></span>}{headline}
      </span></h2>
      {children}
    </div>
    {artwork && <div className="status-artwork">{artwork}</div>}
  </section>;
}
