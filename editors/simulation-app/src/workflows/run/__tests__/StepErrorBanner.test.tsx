/**
 * Tests for StepErrorBanner (P5).
 *
 * Verifies the banner renders only when `stepError` is set, shows the raw
 * backend message (RS002 already names its own offending target — no
 * further parsing needed), and clears the store on dismiss. The
 * store-population side (`useSessionController`'s catch/onError paths
 * calling `setStepError`) is a few lines wired directly into the existing
 * retry/error handling — covered by review, not a separate hook test, to
 * avoid mocking the whole react-query + workspace-store stack for three
 * call sites.
 */

import { afterEach, describe, expect, it } from 'vitest';
import { render, screen, fireEvent, cleanup } from '@testing-library/react';
import { StepErrorBanner } from '../StepErrorBanner';
import { useSessionStore } from '@/features/sessions/store';

afterEach(() => {
  cleanup();
  useSessionStore.getState().clearStepError();
});

describe('StepErrorBanner', () => {
  it('renders nothing when stepError is null', () => {
    const { container } = render(<StepErrorBanner />);
    expect(container.firstChild).toBeNull();
  });

  it('renders the backend error message when stepError is set', () => {
    useSessionStore
      .getState()
      .setStepError(
        "RS002 unknown override target 'OscillatorStateMachine.I_residual': " +
          'resolves to neither a runtime slot alias nor an existing context variable',
      );
    render(<StepErrorBanner />);
    expect(screen.getByTestId('step-error-banner')).toBeInTheDocument();
    expect(screen.getByTestId('step-error-message')).toHaveTextContent('RS002');
    expect(screen.getByTestId('step-error-message')).toHaveTextContent(
      'OscillatorStateMachine.I_residual',
    );
  });

  it('dismiss button clears stepError in the store', () => {
    useSessionStore.getState().setStepError('RS002 unknown override target x');
    render(<StepErrorBanner />);
    fireEvent.click(screen.getByTestId('step-error-dismiss'));
    expect(useSessionStore.getState().stepError).toBeNull();
  });
});
