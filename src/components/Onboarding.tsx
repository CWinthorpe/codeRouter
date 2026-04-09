import { useState, useEffect } from 'react';
import { useNavigate } from 'react-router-dom';
import { ArrowRight, Server, Layers, CheckCircle2 } from 'lucide-react';

interface OnboardingProps {
  providersCount: number;
  groupsCount: number;
  onDismiss: () => void;
}

export function Onboarding({ providersCount, groupsCount, onDismiss }: OnboardingProps) {
  const navigate = useNavigate();
  const [step, setStep] = useState(0);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onDismiss();
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, [onDismiss]);

  const shouldShow = providersCount === 0 || groupsCount === 0;

  if (!shouldShow) return null;

  const steps = [
    {
      title: 'Welcome to CodeRouter',
      description: 'CodeRouter sits between your AI coding tools and multiple LLM providers, providing intelligent failover, cost management, and model grouping.',
      icon: <CheckCircle2 className="h-8 w-8 text-emerald-400" />,
    },
    {
      title: 'Step 1: Add a Provider',
      description: 'Connect your first upstream LLM provider. You can add OpenAI-compatible or Anthropic-compatible providers with API keys stored securely in your system keychain.',
      icon: <Server className="h-8 w-8 text-blue-400" />,
      action: 'Add Provider',
      actionRoute: '/providers',
    },
    {
      title: 'Step 2: Create a Model Group',
      description: 'Once you have providers, create model groups to group models across providers with priority-based failover. This is the core feature of CodeRouter.',
      icon: <Layers className="h-8 w-8 text-purple-400" />,
      action: 'Create Group',
      actionRoute: '/groups',
    },
  ];

  const currentStep = steps[step];

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70" onClick={onDismiss}>
      <div
        role="dialog"
        aria-modal="true"
        className="w-full max-w-lg rounded-xl border border-zinc-700 bg-zinc-900 p-8 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex justify-center mb-6">{currentStep.icon}</div>

        <h2 className="text-xl font-semibold text-center text-zinc-100">{currentStep.title}</h2>
        <p className="mt-3 text-sm text-center text-zinc-400 leading-relaxed">{currentStep.description}</p>

        {/* Progress dots */}
        <div className="flex justify-center gap-2 mt-6">
          {steps.map((_, i) => (
            <div
              key={i}
              className={`h-2 w-2 rounded-full transition-colors ${
                i <= step ? 'bg-emerald-500' : 'bg-zinc-700'
              }`}
            />
          ))}
        </div>

        {/* Actions */}
        <div className="mt-8 flex items-center justify-between">
          {step > 0 ? (
            <button
              onClick={() => setStep(step - 1)}
              className="text-sm text-zinc-400 hover:text-zinc-200 transition-colors"
            >
              Back
            </button>
          ) : (
            <button
              onClick={onDismiss}
              className="text-sm text-zinc-500 hover:text-zinc-300 transition-colors"
            >
              Skip
            </button>
          )}

          {currentStep.action ? (
            <button
              onClick={() => {
                navigate(currentStep.actionRoute!);
                onDismiss();
              }}
              className="flex items-center gap-2 rounded-md bg-emerald-600 px-5 py-2.5 text-sm font-medium text-white transition-colors hover:bg-emerald-500"
            >
              {currentStep.action}
              <ArrowRight className="h-4 w-4" />
            </button>
          ) : (
            <button
              onClick={() => setStep(step + 1)}
              className="flex items-center gap-2 rounded-md bg-zinc-700 px-5 py-2.5 text-sm font-medium text-zinc-200 transition-colors hover:bg-zinc-600"
            >
              Next
              <ArrowRight className="h-4 w-4" />
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
