import React, { useState } from 'react';

interface Props { label: string; children: React.ReactNode; }

export function CollapsibleBlock({ label, children }: Props) {
  const [open, setOpen] = useState(false);
  return (
    <div>
      <div className="collapsible-toggle" onClick={() => setOpen(!open)}>
        {open ? '\u25BC' : '\u25B6'} {label}
      </div>
      <div className={`collapsible-content ${open ? 'open' : ''}`}>{children}</div>
    </div>
  );
}
