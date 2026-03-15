/// Classifies API errors into actionable categories.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ApiErrorKind {
    /// Transient server errors (500, 502, 503) -- retryable
    ServerError,
    /// Rate limit (429) -- retryable with backoff
    RateLimited,
    /// Context window overflow -- not retryable as-is
    ContextOverflow,
    /// Authentication/authorization -- not retryable
    AuthError,
    /// Other non-retryable errors
    Other,
}

impl ApiErrorKind {
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::ServerError | Self::RateLimited)
    }

    /// Return a user-friendly error message.
    pub fn user_message(&self, raw: &str) -> String {
        match self {
            Self::ServerError => format!("API server error (retries exhausted): {raw}"),
            Self::RateLimited => format!("Rate limited by AI provider (retries exhausted): {raw}"),
            Self::ContextOverflow => "The conversation context is too large for the model's context window. Try starting a new conversation or reducing tool output.".to_string(),
            Self::AuthError => "Authentication failed. Check your API key configuration.".to_string(),
            Self::Other => format!("API error: {raw}"),
        }
    }
}

/// Classify an error message string into an ApiErrorKind.
pub fn classify_error(error_msg: &str) -> ApiErrorKind {
    let lower = error_msg.to_lowercase();

    // Context overflow patterns (check first - vLLM returns 500 for these)
    if lower.contains("context_length_exceeded")
        || lower.contains("context length")
        || lower.contains("maximum context")
        || lower.contains("prompt is too long")
        || lower.contains("token limit")
        || lower.contains("max_tokens")
        || lower.contains("too many tokens")
        || lower.contains("exceeds the model")
    {
        return ApiErrorKind::ContextOverflow;
    }

    // Rate limiting
    if lower.contains("429")
        || lower.contains("rate limit")
        || lower.contains("too many requests")
    {
        return ApiErrorKind::RateLimited;
    }

    // Auth errors
    if lower.contains("401")
        || lower.contains("403")
        || lower.contains("unauthorized")
        || lower.contains("forbidden")
        || lower.contains("invalid api key")
        || lower.contains("invalid_api_key")
    {
        return ApiErrorKind::AuthError;
    }

    // Server errors (check after context overflow since vLLM sends 500 for context issues)
    if lower.contains("500")
        || lower.contains("502")
        || lower.contains("503")
        || lower.contains("internal server error")
        || lower.contains("bad gateway")
        || lower.contains("service unavailable")
    {
        return ApiErrorKind::ServerError;
    }

    ApiErrorKind::Other
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_errors() {
        assert_eq!(classify_error("HTTP 500 Internal Server Error"), ApiErrorKind::ServerError);
        assert_eq!(classify_error("502 Bad Gateway"), ApiErrorKind::ServerError);
        assert_eq!(classify_error("503 Service Unavailable"), ApiErrorKind::ServerError);
        assert_eq!(classify_error("internal server error occurred"), ApiErrorKind::ServerError);
        assert_eq!(classify_error("bad gateway from upstream"), ApiErrorKind::ServerError);
        assert_eq!(classify_error("service unavailable, try later"), ApiErrorKind::ServerError);
    }

    #[test]
    fn test_rate_limited() {
        assert_eq!(classify_error("429 Too Many Requests"), ApiErrorKind::RateLimited);
        assert_eq!(classify_error("rate limit exceeded"), ApiErrorKind::RateLimited);
        assert_eq!(classify_error("Too many requests, please slow down"), ApiErrorKind::RateLimited);
    }

    #[test]
    fn test_context_overflow() {
        assert_eq!(classify_error("context_length_exceeded"), ApiErrorKind::ContextOverflow);
        assert_eq!(classify_error("This model's maximum context length is 200000"), ApiErrorKind::ContextOverflow);
        assert_eq!(classify_error("prompt is too long"), ApiErrorKind::ContextOverflow);
        assert_eq!(classify_error("token limit reached"), ApiErrorKind::ContextOverflow);
        assert_eq!(classify_error("too many tokens in request"), ApiErrorKind::ContextOverflow);
        assert_eq!(classify_error("request exceeds the model limit"), ApiErrorKind::ContextOverflow);
        assert_eq!(classify_error("max_tokens exceeded"), ApiErrorKind::ContextOverflow);
    }

    #[test]
    fn test_context_overflow_takes_priority_over_500() {
        // vLLM returns 500 for context overflow -- context overflow should win
        assert_eq!(
            classify_error("500 Internal Server Error: context_length_exceeded"),
            ApiErrorKind::ContextOverflow,
        );
        assert_eq!(
            classify_error("HTTP 500: prompt is too long for this model"),
            ApiErrorKind::ContextOverflow,
        );
    }

    #[test]
    fn test_auth_errors() {
        assert_eq!(classify_error("401 Unauthorized"), ApiErrorKind::AuthError);
        assert_eq!(classify_error("403 Forbidden"), ApiErrorKind::AuthError);
        assert_eq!(classify_error("unauthorized access"), ApiErrorKind::AuthError);
        assert_eq!(classify_error("forbidden resource"), ApiErrorKind::AuthError);
        assert_eq!(classify_error("invalid api key provided"), ApiErrorKind::AuthError);
        assert_eq!(classify_error("invalid_api_key"), ApiErrorKind::AuthError);
    }

    #[test]
    fn test_case_insensitivity() {
        assert_eq!(classify_error("CONTEXT_LENGTH_EXCEEDED"), ApiErrorKind::ContextOverflow);
        assert_eq!(classify_error("Rate Limit Exceeded"), ApiErrorKind::RateLimited);
        assert_eq!(classify_error("UNAUTHORIZED"), ApiErrorKind::AuthError);
        assert_eq!(classify_error("Internal Server Error"), ApiErrorKind::ServerError);
    }

    #[test]
    fn test_unknown_error_returns_other() {
        assert_eq!(classify_error("something went wrong"), ApiErrorKind::Other);
        assert_eq!(classify_error("connection reset by peer"), ApiErrorKind::Other);
        assert_eq!(classify_error(""), ApiErrorKind::Other);
    }

    #[test]
    fn test_retryable() {
        assert!(ApiErrorKind::ServerError.is_retryable());
        assert!(ApiErrorKind::RateLimited.is_retryable());
        assert!(!ApiErrorKind::ContextOverflow.is_retryable());
        assert!(!ApiErrorKind::AuthError.is_retryable());
        assert!(!ApiErrorKind::Other.is_retryable());
    }

    #[test]
    fn test_user_messages() {
        let msg = ApiErrorKind::ServerError.user_message("500 error");
        assert!(msg.contains("retries exhausted"));

        let msg = ApiErrorKind::RateLimited.user_message("429");
        assert!(msg.contains("Rate limited"));

        let msg = ApiErrorKind::ContextOverflow.user_message("too long");
        assert!(msg.contains("context window"));

        let msg = ApiErrorKind::AuthError.user_message("bad key");
        assert!(msg.contains("API key"));

        let msg = ApiErrorKind::Other.user_message("unknown");
        assert!(msg.contains("API error"));
    }
}
