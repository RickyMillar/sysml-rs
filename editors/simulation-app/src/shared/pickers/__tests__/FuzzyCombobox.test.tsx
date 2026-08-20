/**
 * FuzzyCombobox — scoring parity with the palette + basic interaction.
 */
import { describe, it, expect, afterEach } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { FuzzyCombobox, filterCandidates, scoreCandidate } from '../FuzzyCombobox';

afterEach(cleanup);

describe('scoreCandidate / filterCandidates', () => {
  it('ranks startsWith above includes, requires every token, empty query passes all', () => {
    expect(scoreCandidate('motor.torque', 'mot')).toBe(100);
    expect(scoreCandidate('rotor.motion', 'mot')).toBe(40);
    expect(scoreCandidate('motor.torque', 'mot torq')).toBe(140);
    expect(scoreCandidate('motor.torque', 'mot xyz')).toBe(0);
    expect(scoreCandidate('anything', '')).toBe(1);

    const ranked = filterCandidates(['rotor.motion', 'motor.torque', 'brake'], 'mot');
    expect(ranked.map((c) => c.value)).toEqual(['motor.torque', 'rotor.motion']);
  });

  it('detail is searchable at reduced weight and never beats a value match', () => {
    // Typing the MACHINE name surfaces its states…
    expect(scoreCandidate({ value: 'armed', detail: 'state · BreakerStates' }, 'breaker')).toBe(20);
    // …but a state whose VALUE matches ranks above one matching via detail only.
    const ranked = filterCandidates(
      [
        { value: 'armed', detail: 'state · BreakerStates' },
        { value: 'breaker_open', detail: 'state · MainStates' },
      ],
      'breaker',
    );
    expect(ranked.map((c) => c.value)).toEqual(['breaker_open', 'armed']);
    // All-tokens rule spans value + detail together.
    expect(scoreCandidate({ value: 'armed', detail: 'state · BreakerStates' }, 'armed breaker')).toBe(120);
    expect(scoreCandidate({ value: 'armed', detail: 'state · BreakerStates' }, 'armed xyz')).toBe(0);
  });
});

describe('<FuzzyCombobox/>', () => {
  it('shows themed suggestions on focus, filters as typed, picks on Enter', () => {
    let value = '';
    const onChange = (v: string) => { value = v; };
    const { rerender } = render(
      <FuzzyCombobox
        testId="fc"
        value={value}
        onChange={onChange}
        candidates={['motor.torque', 'motor.power', 'brake.force']}
      />,
    );

    const input = screen.getByTestId('fc');
    fireEvent.focus(input);
    expect(screen.getByTestId('fc-suggestions').children).toHaveLength(3);

    fireEvent.change(input, { target: { value: 'motor' } });
    rerender(
      <FuzzyCombobox testId="fc" value="motor" onChange={onChange} candidates={['motor.torque', 'motor.power', 'brake.force']} />,
    );
    expect(screen.getByTestId('fc-suggestions').children).toHaveLength(2);

    fireEvent.keyDown(input, { key: 'ArrowDown' });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(value).toBe('motor.torque');
  });

  it('free-form input stays allowed — no suggestions, no dropdown, value intact', () => {
    render(
      <FuzzyCombobox testId="fc" value="custom_name" onChange={() => {}} candidates={['alpha']} />,
    );
    const input = screen.getByTestId('fc') as HTMLInputElement;
    fireEvent.focus(input);
    expect(screen.queryByTestId('fc-suggestions')).toBeNull();
    expect(input.value).toBe('custom_name');
  });
});
