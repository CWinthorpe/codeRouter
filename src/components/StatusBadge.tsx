/**
 * Badge component that displays a request status label with color-coded
 * styling. Supports "success", "failover", "error", and "timeout" statuses;
 * unknown statuses fall back to error styling.
 */
export function StatusBadge({ status }: { status: string }) {
  // Map known statuses to semantic color classes; unknown values use error style
  const colors: Record<string, string> = {
    success: 'bg-green-500/20 text-green-400 border-green-500/30',
    failover: 'bg-yellow-500/20 text-yellow-400 border-yellow-500/30',
    error: 'bg-red-500/20 text-red-400 border-red-500/30',
    timeout: 'bg-zinc-500/20 text-zinc-400 border-zinc-500/30',
  };
  const color = colors[status] ?? colors.error;
  return (
    <span className={`inline-flex items-center rounded border px-2 py-0.5 text-xs font-medium capitalize ${color}`}>
      {status}
    </span>
  );
}