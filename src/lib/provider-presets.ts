import type { ProviderModel } from '../types';

export interface ProviderPreset {
  id: string;
  name: string;
  baseUrl: string;
  protocol: 'openai' | 'anthropic';
  description: string;
  modelOverrides?: ProviderModel[];
}

export const providerPresets: ProviderPreset[] = [
  {
    id: 'opencode-go',
    name: 'OpenCode Go',
    baseUrl: 'https://opencode.ai/zen/go/v1',
    protocol: 'openai',
    description: '$10/mo subscription for curated open coding models',
    modelOverrides: [
      { id: 'glm-5.1', context_window: 202752, max_output_tokens: 65535 },
      { id: 'glm-5', context_window: 198000, max_output_tokens: 131072 },
      { id: 'kimi-k2.5', context_window: 262144, max_output_tokens: 65535 },
      { id: 'mimo-v2-pro', context_window: 1048576, max_output_tokens: 131072 },
      { id: 'mimo-v2-omni', context_window: 262144, max_output_tokens: 65536 },
      { id: 'minimax-m2.7', context_window: 204800, max_output_tokens: 131072, protocol: 'anthropic' },
      { id: 'minimax-m2.5', context_window: 196608, max_output_tokens: 65536, protocol: 'anthropic' },
    ],
  },
  {
    id: 'opencode-zen',
    name: 'OpenCode Zen',
    baseUrl: 'https://opencode.ai/zen/v1',
    protocol: 'openai',
    description: 'Pay-as-you-go gateway to verified models',
    modelOverrides: [
      { id: 'claude-opus-4-6', context_window: 1000000, max_output_tokens: 128000, protocol: 'anthropic' },
      { id: 'claude-opus-4-5', context_window: 200000, max_output_tokens: 64000, protocol: 'anthropic' },
      { id: 'claude-opus-4-1', context_window: 200000, max_output_tokens: 32000, protocol: 'anthropic' },
      { id: 'claude-sonnet-4-6', context_window: 1000000, max_output_tokens: 128000, protocol: 'anthropic' },
      { id: 'claude-sonnet-4-5', context_window: 1000000, max_output_tokens: 64000, protocol: 'anthropic' },
      { id: 'claude-sonnet-4', context_window: 1000000, max_output_tokens: 64000, protocol: 'anthropic' },
      { id: 'claude-haiku-4-5', context_window: 200000, max_output_tokens: 64000, protocol: 'anthropic' },
      { id: 'claude-3-5-haiku', context_window: 200000, max_output_tokens: 8192, protocol: 'anthropic' },
      { id: 'gemini-3.1-pro', context_window: 1048576, max_output_tokens: 65536 },
      { id: 'gemini-3-flash', context_window: 1048576, max_output_tokens: 65536 },
      { id: 'glm-5.1', context_window: 202752, max_output_tokens: 65535 },
      { id: 'glm-5', context_window: 198000, max_output_tokens: 131072 },
      { id: 'kimi-k2.5', context_window: 262144, max_output_tokens: 65535 },
      { id: 'minimax-m2.5', context_window: 196608, max_output_tokens: 65536 },
      { id: 'gpt-5.4', context_window: 1050000, max_output_tokens: 128000 },
      { id: 'gpt-5.4-mini', context_window: 400000, max_output_tokens: 128000 },
      { id: 'gpt-5.4-nano', context_window: 400000, max_output_tokens: 128000 },
      { id: 'big-pickle' },
      { id: 'qwen3.6-plus-free', context_window: 1000000, max_output_tokens: 65536 },
      { id: 'nemotron-3-super-free', context_window: 262144, max_output_tokens: 262144 },
      { id: 'minimax-m2.5-free', context_window: 196608, max_output_tokens: 196608 },
      { id: 'gpt-5-nano', context_window: 400000, max_output_tokens: 400000 },
    ],
  },
  {
    id: 'openrouter',
    name: 'OpenRouter',
    baseUrl: 'https://openrouter.ai/api/v1',
    protocol: 'openai',
    description: 'Unified API for 300+ models across providers',
  },
  {
    id: 'openai',
    name: 'OpenAI',
    baseUrl: 'https://api.openai.com/v1',
    protocol: 'openai',
    description: 'GPT models directly from OpenAI',
  },
  {
    id: 'anthropic',
    name: 'Anthropic',
    baseUrl: 'https://api.anthropic.com',
    protocol: 'anthropic',
    description: 'Claude models directly from Anthropic',
  },
  {
    id: 'google-ai',
    name: 'Google AI',
    baseUrl: 'https://generativelanguage.googleapis.com/v1beta/openai',
    protocol: 'openai',
    description: 'Gemini models via OpenAI-compatible endpoint',
  },
];
