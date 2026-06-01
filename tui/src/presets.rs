use coderouter_proxy::config::models::ProviderModel;

pub struct ProviderPreset {
    pub id: &'static str,
    pub name: &'static str,
    pub base_url: &'static str,
    pub protocol: &'static str,
    pub description: &'static str,
    pub model_overrides: Vec<ProviderModel>,
}

fn mo(id: &str) -> ProviderModel {
    ProviderModel {
        id: id.to_string(),
        context_window: None,
        max_output_tokens: None,
        input_cost_per_1m: None,
        output_cost_per_1m: None,
        last_refreshed: None,
        protocol: None,
    }
}

fn mo_ctx(id: &str, ctx: u64, max_out: u64) -> ProviderModel {
    ProviderModel {
        id: id.to_string(),
        context_window: Some(ctx),
        max_output_tokens: Some(max_out),
        input_cost_per_1m: None,
        output_cost_per_1m: None,
        last_refreshed: None,
        protocol: None,
    }
}

fn mo_full(id: &str, ctx: u64, max_out: u64, inp: f64, out: f64) -> ProviderModel {
    ProviderModel {
        id: id.to_string(),
        context_window: Some(ctx),
        max_output_tokens: Some(max_out),
        input_cost_per_1m: Some(inp),
        output_cost_per_1m: Some(out),
        last_refreshed: None,
        protocol: None,
    }
}

fn mo_proto(id: &str, ctx: u64, max_out: u64, proto: &str) -> ProviderModel {
    ProviderModel {
        id: id.to_string(),
        context_window: Some(ctx),
        max_output_tokens: Some(max_out),
        input_cost_per_1m: None,
        output_cost_per_1m: None,
        last_refreshed: None,
        protocol: Some(proto.to_string()),
    }
}

fn mo_full_proto(
    id: &str,
    ctx: u64,
    max_out: u64,
    inp: f64,
    out: f64,
    proto: &str,
) -> ProviderModel {
    ProviderModel {
        id: id.to_string(),
        context_window: Some(ctx),
        max_output_tokens: Some(max_out),
        input_cost_per_1m: Some(inp),
        output_cost_per_1m: Some(out),
        last_refreshed: None,
        protocol: Some(proto.to_string()),
    }
}

pub fn provider_presets() -> Vec<ProviderPreset> {
    vec![
        ProviderPreset {
            id: "opencode-go",
            name: "OpenCode Go",
            base_url: "https://opencode.ai/zen/go/v1",
            protocol: "openai",
            description: "$10/mo subscription for curated open coding models",
            model_overrides: vec![
                mo_ctx("glm-5.1", 202752, 65535),
                mo_ctx("glm-5", 198000, 131072),
                mo_ctx("kimi-k2.5", 262144, 65535),
                mo_ctx("mimo-v2-pro", 1048576, 131072),
                mo_ctx("mimo-v2-omni", 262144, 65536),
                mo_proto("minimax-m2.7", 204800, 131072, "anthropic"),
                mo_proto("minimax-m2.5", 196608, 65536, "anthropic"),
                mo_ctx("qwen3.6-plus", 1000000, 65536),
                mo_ctx("qwen3.5-plus", 262144, 65536),
                mo_ctx("kimi-k2.6", 262144, 262144),
                mo_ctx("deepseek-v4-pro", 1048576, 131072),
                mo_ctx("deepseek-v4-flash", 1048576, 131072),
                mo_ctx("mimo-v2.5-pro", 1048576, 131072),
                mo_ctx("mimo-v2.5", 1048576, 65536),
            ],
        },
        ProviderPreset {
            id: "opencode-zen",
            name: "OpenCode Zen",
            base_url: "https://opencode.ai/zen/v1",
            protocol: "openai",
            description: "Pay-as-you-go gateway to verified models",
            model_overrides: vec![
                mo_full_proto("claude-opus-4-6", 1000000, 128000, 5.0, 25.0, "anthropic"),
                mo_full_proto("claude-opus-4-5", 200000, 64000, 5.0, 25.0, "anthropic"),
                mo_full_proto("claude-opus-4-1", 200000, 32000, 15.0, 75.0, "anthropic"),
                mo_full_proto("claude-sonnet-4-6", 1000000, 128000, 3.0, 15.0, "anthropic"),
                mo_full_proto("claude-sonnet-4-5", 1000000, 64000, 3.0, 15.0, "anthropic"),
                mo_full_proto("claude-sonnet-4", 1000000, 64000, 3.0, 15.0, "anthropic"),
                mo_full_proto("claude-haiku-4-5", 200000, 64000, 1.0, 5.0, "anthropic"),
                mo_full_proto("claude-3-5-haiku", 200000, 8192, 0.8, 4.0, "anthropic"),
                mo_full("gemini-3.1-pro", 1048576, 65536, 2.0, 12.0),
                mo_full("gemini-3-flash", 1048576, 65536, 0.5, 3.0),
                mo_full("glm-5.1", 202752, 65535, 1.4, 4.4),
                mo_full("glm-5", 198000, 131072, 1.0, 3.2),
                mo_full("kimi-k2.5", 262144, 65535, 0.6, 3.0),
                mo_full("minimax-m2.5", 196608, 65536, 0.3, 1.2),
                mo_full("gpt-5.4", 1050000, 128000, 2.5, 15.0),
                mo_full("gpt-5.4-mini", 400000, 128000, 0.75, 4.5),
                mo_full("gpt-5.4-nano", 400000, 128000, 0.2, 1.25),
                mo("big-pickle"),
                mo_ctx("qwen3.6-plus-free", 1000000, 65536),
                mo_ctx("nemotron-3-super-free", 262144, 262144),
                mo_ctx("minimax-m2.5-free", 196608, 196608),
                mo_ctx("gpt-5-nano", 400000, 400000),
                mo_full_proto("claude-opus-4-7", 1000000, 128000, 5.0, 25.0, "anthropic"),
                mo_full("kimi-k2.6", 262144, 65535, 0.95, 4.0),
                mo_full("minimax-m2.7", 196608, 131072, 0.30, 1.20),
                mo_full("gpt-5.5", 1050000, 128000, 5.0, 30.0),
                mo_full("gpt-5.5-pro", 1050000, 128000, 30.0, 180.0),
                mo_full("gpt-5.4-pro", 1050000, 128000, 30.0, 180.0),
                mo_full("gpt-5.3-codex", 400000, 128000, 1.75, 14.0),
                mo_full("gpt-5.3-codex-spark", 400000, 128000, 1.75, 14.0),
                mo_full("gpt-5.2", 400000, 128000, 1.75, 14.0),
                mo_full("gpt-5.2-codex", 400000, 128000, 1.75, 14.0),
                mo_full("gpt-5.1", 400000, 128000, 1.07, 8.50),
                mo_full("gpt-5.1-codex", 400000, 128000, 1.07, 8.50),
                mo_full("gpt-5.1-codex-max", 400000, 128000, 1.25, 10.0),
                mo_full("gpt-5.1-codex-mini", 400000, 128000, 0.25, 2.0),
                mo_full("gpt-5", 400000, 128000, 1.07, 8.50),
                mo_full("gpt-5-codex", 400000, 128000, 1.07, 8.50),
                mo_ctx("ling-2.6-flash", 262144, 262144),
                mo_ctx("hy3-preview-free", 262144, 262144),
                mo_full("qwen3.6-plus", 1000000, 65536, 0.50, 3.0),
                mo_full("qwen3.5-plus", 1000000, 65536, 0.20, 1.20),
            ],
        },
        ProviderPreset {
            id: "zai-coding-plan",
            name: "Z.AI Coding Plan",
            base_url: "https://api.z.ai/api/coding/paas/v4",
            protocol: "openai",
            description: "Subscription plan for GLM coding models",
            model_overrides: vec![
                mo_ctx("glm-5.1", 202752, 65535),
                mo_ctx("glm-5v-turbo", 202752, 131072),
                mo_ctx("glm-5-turbo", 202752, 131072),
                mo_ctx("glm-5", 198000, 131072),
                mo_ctx("glm-4.7", 202752, 65535),
                mo_ctx("glm-4.7-flash", 202752, 202752),
                mo_ctx("glm-4.7-flashx", 202752, 202752),
                mo_ctx("glm-4.6v", 131072, 131072),
                mo_ctx("glm-4.6", 204800, 204800),
                mo_ctx("glm-4.5v", 65536, 16384),
                mo_ctx("glm-4.5", 131072, 98304),
                mo_ctx("glm-4.5-flash", 131072, 98304),
                mo_ctx("glm-4.5-air", 131072, 98304),
            ],
        },
        ProviderPreset {
            id: "venice",
            name: "Venice AI",
            base_url: "https://api.venice.ai/api/v1",
            protocol: "openai",
            description: "Privacy-focused AI with no data logging",
            model_overrides: vec![],
        },
        ProviderPreset {
            id: "openrouter",
            name: "OpenRouter",
            base_url: "https://openrouter.ai/api/v1",
            protocol: "openai",
            description: "Unified API for 300+ models across providers",
            model_overrides: vec![],
        },
        ProviderPreset {
            id: "openai",
            name: "OpenAI",
            base_url: "https://api.openai.com/v1",
            protocol: "openai",
            description: "GPT models directly from OpenAI",
            model_overrides: vec![],
        },
        ProviderPreset {
            id: "openai-codex",
            name: "OpenAI Codex",
            base_url: "https://chatgpt.com/backend-api/codex",
            protocol: "openai-codex",
            description: "Codex GPT models via ChatGPT device login",
            model_overrides: vec![],
        },
        ProviderPreset {
            id: "anthropic",
            name: "Anthropic",
            base_url: "https://api.anthropic.com",
            protocol: "anthropic",
            description: "Claude models directly from Anthropic",
            model_overrides: vec![
                mo_full("claude-opus-4-7", 1000000, 128000, 5.0, 25.0),
                mo_full("claude-opus-4-6", 1000000, 128000, 5.0, 25.0),
                mo_full("claude-opus-4-5", 200000, 64000, 5.0, 25.0),
                mo_full("claude-opus-4-1", 200000, 32000, 15.0, 75.0),
                mo_full("claude-sonnet-4-6", 1000000, 128000, 3.0, 15.0),
                mo_full("claude-sonnet-4-5", 1000000, 64000, 3.0, 15.0),
                mo_full("claude-sonnet-4", 1000000, 64000, 3.0, 15.0),
                mo_full("claude-haiku-4-5", 200000, 64000, 1.0, 5.0),
                mo_full("claude-3-5-haiku", 200000, 8192, 0.8, 4.0),
                mo_full("claude-3-haiku", 200000, 4096, 0.25, 1.25),
                mo_full("claude-3.7-sonnet", 200000, 128000, 3.0, 15.0),
            ],
        },
        ProviderPreset {
            id: "google-ai",
            name: "Google AI",
            base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
            protocol: "openai",
            description: "Gemini models via OpenAI-compatible endpoint",
            model_overrides: vec![],
        },
        ProviderPreset {
            id: "crofai",
            name: "CrofAI",
            base_url: "https://crof.ai/v1",
            protocol: "openai",
            description:
                "World's cheapest inference — quantized OSS models, zero data retention, from $5/mo",
            model_overrides: vec![],
        },
    ]
}
