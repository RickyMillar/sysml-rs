/**
 * Framed-view layer (§8.2.3.26 / contract §C, task C6): frame border,
 * «view» heading tab, corner info compartments — geometry + rendering.
 */
import { describe, expect, it, afterEach } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import {
  FRAME_HEADING_H,
  FRAME_PAD,
  FRAME_SLOT_H,
  ViewFrameLayer,
  frameExtents,
  frameHeadingParts,
} from '../frame';
import type { ViewFrame } from '../viewmodel-types';

const palette = { bg: '#1A140F', muted: '#8A7D68', text: '#EFE8DC' };

const frame = (over: Partial<ViewFrame> = {}): ViewFrame => ({
  view_kind: 'General',
  name: 'OverviewView',
  type_name: null,
  top_right: null,
  bottom_left: null,
  bottom_right: null,
  ...over,
});

afterEach(cleanup);

describe('frameExtents', () => {
  it('pads the content box and reserves the heading strip on top', () => {
    const e = frameExtents({ width: 400, height: 300 }, frame());
    expect(e).toEqual({
      x0: -FRAME_PAD,
      y0: -(FRAME_PAD + FRAME_HEADING_H),
      x1: 400 + FRAME_PAD,
      y1: 300 + FRAME_PAD,
    });
  });

  it('reserves a bottom strip only when bottom info slots exist', () => {
    const e = frameExtents({ width: 400, height: 300 }, frame({ bottom_left: { text: 'filter x' } }));
    expect(e.y1).toBe(300 + FRAME_PAD + FRAME_SLOT_H);
  });
});

describe('frameHeadingParts', () => {
  it('suffixes the LITERAL declared type, not the render-word (R7, §8.2.3.26)', () => {
    // `view def DriveModesView :> StateTransition` → the declared supertype is
    // the heading suffix, even though the view renders through the State kind.
    expect(
      frameHeadingParts(frame({ view_kind: 'StateTransition', name: 'DriveModesView', type_name: 'StateTransition' })),
    ).toEqual({
      main: '«view» DriveModesView',
      suffix: ' : StateTransition',
    });
  });

  it('omits the suffix when the view declares no type (never synthesizes the kind)', () => {
    // A view with no declared supertype: `«view» Name` with NO ` : General`.
    expect(frameHeadingParts(frame({ view_kind: 'General', name: 'OverviewView', type_name: null }))).toEqual({
      main: '«view» OverviewView',
      suffix: '',
    });
  });
});

describe('ViewFrameLayer', () => {
  const renderLayer = (f: ViewFrame) =>
    render(
      <svg>
        <ViewFrameLayer frame={f} width={400} height={300} palette={palette} />
      </svg>,
    );

  it('draws the frame border around the content with heading text', () => {
    renderLayer(frame({ type_name: 'InterconnectionView' }));
    const border = screen.getByTestId('svgc-view-frame-border');
    expect(border.getAttribute('x')).toBe(String(-FRAME_PAD));
    expect(border.getAttribute('y')).toBe(String(-(FRAME_PAD + FRAME_HEADING_H)));
    expect(border.getAttribute('width')).toBe(String(400 + 2 * FRAME_PAD));
    expect(border.getAttribute('fill')).toBe('none');
    expect(screen.getByText('«view» OverviewView')).toBeTruthy();
    // The suffix is the declared type_name (R7), not the internal view_kind.
    expect(screen.getByText(/: InterconnectionView/)).toBeTruthy();
  });

  it('renders each corner info compartment only when present', () => {
    renderLayer(
      frame({
        top_right: { text: 'expose Vehicle::**' },
        bottom_left: { text: 'filter @Safety' },
      }),
    );
    expect(screen.getByText('expose Vehicle::**')).toBeTruthy();
    expect(screen.getByText('filter @Safety')).toBeTruthy();
    // No bottom_right slot → exactly 3 text elements (heading + 2 slots).
    expect(screen.getByTestId('svgc-view-frame').querySelectorAll('text')).toHaveLength(3);
  });
});
