import { CheckCircle2, XCircle } from 'lucide-react';

export function Toast({ type, message }: { type: 'success' | 'error'; message: string }) {
  const bgColor = type === 'success' ? 'bg-emerald-600' : 'bg-red-600';
  const icon = type === 'success' ? <CheckCircle2 className="h-4 w-4" /> : <XCircle className="h-4 w-4" />;

  return (
    <div className={`${bgColor} flex items-center gap-2 rounded-md px-4 py-3 text-sm text-white shadow-lg`}>
      {icon}
      <span>{message}</span>
    </div>
  );
}
