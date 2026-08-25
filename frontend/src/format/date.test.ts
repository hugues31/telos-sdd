import { describe, expect, test } from 'vitest';

import { formatLocalDate } from './date';

describe('local calendar-date formatting', () => {
  test('uses the requested locale without shifting the calendar date', () => {
    expect(formatLocalDate('2026-08-25', 'en-GB')).toBe('25 Aug 2026');
    expect(formatLocalDate('2024-02-29', 'fr-FR')).toMatch(/29.*févr.*2024/i);
  });

  test('returns the source value for invalid calendar dates and text', () => {
    expect(formatLocalDate('2026-02-29', 'en-GB')).toBe('2026-02-29');
    expect(formatLocalDate('not-a-date', 'en-GB')).toBe('not-a-date');
  });
});
