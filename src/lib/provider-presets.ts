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
      { id: 'glm-5.1' },
      { id: 'glm-5' },
      { id: 'kimi-k2.5' },
      { id: 'mimo-v2-pro' },
      { id: 'mimo-v2-omni' },
      { id: 'minimax-m2.7', protocol: 'anthropic' },
      { id: 'minimax-m2.5', protocol: 'anthropic' },
    ],
  },
  {
    id: 'opencode-zen',
    name: 'OpenCode Zen',
    baseUrl: 'https://opencode.ai/zen/v1',
    protocol: 'openai',
    description: 'Pay-as-you-go gateway to verified models',
    modelOverrides: [
      { id: 'claude-opus-4-6', protocol: 'anthropic' },
      { id: 'claude-opus-4-5', protocol: 'anthropic' },
      { id: 'claude-opus-4-1', protocol: 'anthropic' },
      { id: 'claude-sonnet-4-6', protocol: 'anthropic' },
      { id: 'claude-sonnet-4-5', protocol: 'anthropic' },
      { id: 'claude-sonnet-4', protocol: 'anthropic' },
      { id: 'claude-haiku-4-5', protocol: 'anthropic' },
      { id: 'claude-3-5-haiku', protocol: 'anthropic' },
      { id: 'gemini-3.1-pro' },
      { id: 'gemini-3-flash' },
      { id: 'glm-5.1' },
      { id: 'glm-5' },
      { id: 'kimi-k2.5' },
      { id: 'minimax-m2.5' },
      { id: 'gpt-5.4' },
      { id: 'gpt-5.4-mini' },
      { id: 'gpt-5.4-nano' },
      { id: 'big-pickle' },
      { id: 'qwen3.6-plus-free' },
      { id: 'nemotron-3-super-free' },
      { id: 'minimax-m2.5-free' },
      { id: 'gpt-5-nano' },
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
