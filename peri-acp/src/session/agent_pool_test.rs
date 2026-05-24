use super::*;

fn make_openai_provider(model: &str) -> LlmProvider {
    LlmProvider::OpenAi {
        api_key: "test-key".to_string(),
        base_url: "https://api.example.com/v1".to_string(),
        model: model.to_string(),
        thinking: None,
    }
}

fn make_anthropic_provider(model: &str) -> LlmProvider {
    LlmProvider::Anthropic {
        api_key: "test-key".to_string(),
        model: model.to_string(),
        base_url: None,
        thinking: None,
    }
}

#[test]
fn test_agent_pool_new_is_empty() {
    let pool = AgentPool::new();
    assert!(pool.get_cached_llm().is_none());
    assert!(pool.fingerprint().is_empty());
}

#[test]
fn test_has_valid_cache_empty_pool() {
    let pool = AgentPool::new();
    let provider = make_openai_provider("gpt-4o");
    assert!(!pool.has_valid_cache(&provider));
}

#[test]
fn test_invalidate_clears_cache() {
    let mut pool = AgentPool::new();
    pool.fingerprint = "OpenAI:gpt-4o".to_string();
    pool.invalidate();
    assert!(pool.get_cached_llm().is_none());
    assert!(pool.fingerprint().is_empty());
}

#[test]
fn test_has_valid_cache_fingerprint_mismatch() {
    let mut pool = AgentPool::new();
    // 模拟已缓存但 fingerprint 不匹配
    pool.fingerprint = "OpenAI:gpt-4o".to_string();
    // cached_llm 为 None，has_valid_cache 应返回 false
    let provider = make_openai_provider("gpt-4o");
    assert!(!pool.has_valid_cache(&provider));
}

#[test]
fn test_fingerprint_openai() {
    let provider = make_openai_provider("gpt-4o-mini");
    let fp = fingerprint(&provider);
    assert_eq!(fp, "OpenAI:gpt-4o-mini");
}

#[test]
fn test_fingerprint_anthropic() {
    let provider = make_anthropic_provider("claude-sonnet-4-20250514");
    let fp = fingerprint(&provider);
    assert_eq!(fp, "Anthropic:claude-sonnet-4-20250514");
}

#[test]
fn test_has_valid_cache_after_fingerprint_only_set() {
    let mut pool = AgentPool::new();
    // 直接设置 fingerprint 但没有 cached_llm
    pool.fingerprint = "OpenAI:gpt-4o".to_string();
    let provider = make_openai_provider("gpt-4o");
    // cached_llm 为 None → false
    assert!(!pool.has_valid_cache(&provider));
}
