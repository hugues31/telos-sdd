export function formatLocalDate(value: string, locales?: Intl.LocalesArgument): string {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!match) return value;

  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  const date = new Date(year, month - 1, day);

  if (
    date.getFullYear() !== year ||
    date.getMonth() !== month - 1 ||
    date.getDate() !== day
  ) {
    return value;
  }

  try {
    return new Intl.DateTimeFormat(locales, { dateStyle: 'medium' }).format(date);
  } catch {
    return value;
  }
}
