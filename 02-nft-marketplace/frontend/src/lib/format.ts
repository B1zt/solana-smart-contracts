/** Format a raw token amount for display, given its decimals. */
export function formatAmount(raw: string | bigint, decimals = 9): string {
  const value = typeof raw === 'string' ? BigInt(raw) : raw;
  if (value === 0n) return '0';

  const divisor = 10n ** BigInt(decimals);
  const whole = value / divisor;
  const fraction = value % divisor;

  if (fraction === 0n) return whole.toLocaleString();

  const fractionText = fraction.toString().padStart(decimals, '0').replace(/0+$/, '');
  return `${whole.toLocaleString()}.${fractionText.slice(0, 4)}`;
}

/** Parse a user-typed amount into base units. Returns null rather than throwing on partial input. */
export function parseAmount(input: string, decimals = 9): bigint | null {
  const trimmed = input.trim();
  if (!trimmed || !/^\d*\.?\d*$/.test(trimmed)) return null;

  const [whole = '0', fraction = ''] = trimmed.split('.');
  if (fraction.length > decimals) return null;

  try {
    return BigInt(whole || '0') * 10n ** BigInt(decimals) + BigInt(fraction.padEnd(decimals, '0') || '0');
  } catch {
    return null;
  }
}

/** `abcd…wxyz`, the standard truncation for Solana addresses. */
export function shortAddress(address: string): string {
  if (address.length < 12) return address;
  return `${address.slice(0, 4)}…${address.slice(-4)}`;
}

/** `just now`, `12m ago`, `3d ago`. For timestamps in the past, such as sale history. */
export function formatRelativeTime(target: Date | string | number): string {
  const elapsed = Date.now() - new Date(target).getTime();
  if (elapsed < 60_000) return 'just now';

  const minutes = Math.floor(elapsed / 60_000);
  if (minutes < 60) return `${minutes}m ago`;

  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;

  const days = Math.floor(hours / 24);
  if (days < 30) return `${days}d ago`;

  return new Date(target).toLocaleDateString();
}

/** `2d 4h`, `3h 12m`, `45s`. Null once the deadline has passed. */
export function formatCountdown(target: Date | string | number): string | null {
  const remaining = new Date(target).getTime() - Date.now();
  if (remaining <= 0) return null;

  const seconds = Math.floor(remaining / 1000);
  const days = Math.floor(seconds / 86_400);
  const hours = Math.floor((seconds % 86_400) / 3_600);
  const minutes = Math.floor((seconds % 3_600) / 60);

  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  if (minutes > 0) return `${minutes}m ${seconds % 60}s`;

  return `${seconds}s`;
}
