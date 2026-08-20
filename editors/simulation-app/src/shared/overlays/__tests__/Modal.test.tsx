/**
 * Tests for the Modal primitive (ninebar Phase 1, task #9).
 *
 * Covers: nothing renders when closed, `role="dialog"` + `aria-modal`
 * when open, focus moves to the first focusable element on open, Tab
 * cycles (wraps) within the panel, Escape and backdrop-click both close,
 * a click inside the panel does not close, and focus is restored to the
 * trigger element on close.
 */
import { useState } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Modal } from '../Modal';

afterEach(cleanup);

describe('Modal', () => {
  it('renders nothing when closed', () => {
    const { container } = render(
      <Modal open={false} onClose={() => {}} title="Test">
        body
      </Modal>,
    );
    expect(container.firstChild).toBeNull();
  });

  it('renders role="dialog" + aria-modal when open', () => {
    render(
      <Modal open onClose={() => {}} title="Test">
        <button>ok</button>
      </Modal>,
    );
    const dialog = screen.getByRole('dialog', { name: 'Test' });
    expect(dialog).toBeInTheDocument();
    expect(dialog).toHaveAttribute('aria-modal', 'true');
  });

  it('focuses the first focusable element on open', async () => {
    render(
      <Modal open onClose={() => {}} title="Test">
        <button>first</button>
        <button>second</button>
      </Modal>,
    );
    await waitFor(() => expect(screen.getByText('first')).toHaveFocus());
  });

  it('Escape calls onClose', () => {
    const onClose = vi.fn();
    render(
      <Modal open onClose={onClose} title="Test">
        body
      </Modal>,
    );
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('backdrop click calls onClose', () => {
    const onClose = vi.fn();
    render(
      <Modal open onClose={onClose} title="Test">
        body
      </Modal>,
    );
    fireEvent.click(screen.getByTestId('modal-backdrop'));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('a click inside the panel does not close', () => {
    const onClose = vi.fn();
    render(
      <Modal open onClose={onClose} title="Test">
        <button>ok</button>
      </Modal>,
    );
    fireEvent.click(screen.getByText('ok'));
    expect(onClose).not.toHaveBeenCalled();
  });

  // The Tab-cycle spans the WHOLE panel (header + body), so the modal's
  // own close button is a real stop in the loop — it precedes the body
  // content in DOM order, giving the tab sequence
  // [modal-close, "first", "last"]. This is deliberately different from
  // *initial* focus (which is scoped to the body content only — see the
  // "focuses the first focusable element" test above).

  it('Tab wraps focus from the last element in the panel back to the close button', () => {
    render(
      <Modal open onClose={() => {}} title="Test">
        <button>first</button>
        <button>last</button>
      </Modal>,
    );
    const closeBtn = screen.getByTestId('modal-close');
    screen.getByText('last').focus();
    fireEvent.keyDown(screen.getByTestId('modal-panel'), { key: 'Tab' });
    expect(closeBtn).toHaveFocus();
  });

  it('Shift+Tab wraps focus from the close button back to the last element in the panel', () => {
    render(
      <Modal open onClose={() => {}} title="Test">
        <button>first</button>
        <button>last</button>
      </Modal>,
    );
    const closeBtn = screen.getByTestId('modal-close');
    const last = screen.getByText('last');
    closeBtn.focus();
    fireEvent.keyDown(screen.getByTestId('modal-panel'), { key: 'Tab', shiftKey: true });
    expect(last).toHaveFocus();
  });

  it('restores focus to the previously-active element on close', async () => {
    function Wrapper() {
      const [open, setOpen] = useState(false);
      return (
        <>
          <button data-testid="opener" onClick={() => setOpen(true)}>
            open
          </button>
          <Modal open={open} onClose={() => setOpen(false)} title="Test">
            <button>inside</button>
          </Modal>
        </>
      );
    }
    render(<Wrapper />);
    const opener = screen.getByTestId('opener');
    opener.focus();
    fireEvent.click(opener);
    await waitFor(() => expect(screen.getByText('inside')).toHaveFocus());
    fireEvent.keyDown(window, { key: 'Escape' });
    await waitFor(() => expect(opener).toHaveFocus());
  });
});
