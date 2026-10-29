//! Shared helpers for `LlmProvider` implementations.

/// Strips a Markdown code fence (```` ```json ```` or bare ```` ``` ````) around a JSON reply.
pub(crate) fn strip_fence(s: &str) -> &str {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("```") {
        let rest = rest.strip_suffix("```").unwrap_or(rest);
        let rest = rest.strip_prefix("json").unwrap_or(rest);
        return rest.trim();
    }
    s
}
