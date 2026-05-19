import React from 'react';
import type { CrossAgentInteraction } from '../types';

interface Props { interaction: CrossAgentInteraction; }

export function InteractionArrow({ interaction }: Props) {
  return (
    <div className="interaction-arrow">
      <div className="arrow-line" />
      <div className="arrow-label">{interaction.label}</div>
      <div className="arrow-line" />
    </div>
  );
}
