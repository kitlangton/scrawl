import { useEffect, useRef, useState, type ReactNode } from 'react';
import { Button } from './button';
/** Ordinary button navigation, not an ARIA menu: Tab and native focus still work. */
export function DisclosureMenu({ label, trigger, children }: { label: string; trigger: ReactNode; children: (close: () => void) => ReactNode }) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null), button = useRef<HTMLButtonElement>(null);
  const close = () => { setOpen(false); button.current?.focus(); };
  useEffect(() => {
    if (!open) return;
    const outside = (event: PointerEvent) => { if (!root.current?.contains(event.target as Node)) setOpen(false); };
    document.addEventListener('pointerdown', outside);
    return () => document.removeEventListener('pointerdown', outside);
  }, [open]);
  return <div className="apparent-disclosure" ref={root} onKeyDown={event => { if (event.key === 'Escape' && open) { event.stopPropagation(); close(); } }}>
    <Button ref={button} variant="outline" aria-label={label} aria-expanded={open} onClick={() => setOpen(!open)}>{trigger}<span aria-hidden="true">⌄</span></Button>
    {open && <div className="disclosure-panel" aria-label={label}>{children(close)}</div>}
  </div>;
}
