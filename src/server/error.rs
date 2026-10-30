use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

pub type Result<T> = std::result::Result<T, Error>;

/// A `Result` whose error is boxed by default.
///
/// `Error` carries `reqwest::Error`, which is large, so a function returning `Result<T, Error>`
/// by value is a `clippy::result_large_err` site. Boxing keeps the returned error small without
/// changing what the caller sees: the `Box` is transparent through `Display` and `Error`.
pub type BoxedResult<T, E = Box<Error>> = std::result::Result<T, E>;

impl From<Box<Error>> for Error {
    fn from(boxed: Box<Error>) -> Self {
        *boxed
    }
}

/// Rich error context for debugging and recovery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorContext {
    /// Unique error ID for tracking
    pub error_id: String,
    /// Timestamp when error occurred
    pub timestamp: u64,
    /// The operation that was being performed
    pub operation: String,
    /// Component or module where error occurred
    pub component: String,
    /// Stack of error causes
    pub error_chain: Vec<String>,
    /// Additional context data
    pub context_data: HashMap<String, String>,
    /// Recovery suggestions
    pub recovery_suggestions: Vec<String>,
    /// Whether the operation can be retried
    pub is_retryable: bool,
    /// Severity level
    pub severity: ErrorSeverity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ErrorSeverity {
    Info,
    Warning,
    Error,
    Critical,
}

impl ErrorContext {
    pub fn new(operation: &str, component: &str) -> Self {
        // Generate error ID with fallback
        let error_id = match uuid::Uuid::try_parse("00000000-0000-0000-0000-000000000000") {
            Ok(_) => {
                // UUID library is working, use new_v4
                uuid::Uuid::new_v4().to_string()
            }
            Err(_) => {
                // Fallback to timestamp-based ID
                format!("err_{}", 
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_nanos())
                        .unwrap_or_else(|_| {
                            // Final fallback to process-based ID
                            std::process::id() as u128 * 1_000_000
                        })
                )
            }
        };

        Self {
            error_id,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_else(|_| {
                    tracing::warn!("Failed to get system time for error context, using epoch");
                    std::time::Duration::from_secs(0)
                })
                .as_secs(),
            operation: operation.to_string(),
            component: component.to_string(),
            error_chain: Vec::new(),
            context_data: HashMap::new(),
            recovery_suggestions: Vec::new(),
            is_retryable: false,
            severity: ErrorSeverity::Error,
        }
    }

    pub fn add_cause(mut self, cause: &str) -> Self {
        self.error_chain.push(cause.to_string());
        self
    }

    pub fn add_context(mut self, key: &str, value: &str) -> Self {
        // Enhanced sanitization for sensitive data
        let sanitized_value = if Self::is_sensitive_key(key) {
            "[REDACTED]".to_string()
        } else {
            value.to_string()
        };
        self.context_data.insert(key.to_string(), sanitized_value);
        self
    }

    /// Check if a key contains sensitive information
    fn is_sensitive_key(key: &str) -> bool {
        let key_lower = key.to_lowercase();
        let sensitive_patterns = [
            "password", "passwd", "pwd",
            "token", "auth", "authorization", "bearer",
            "secret", "key", "api_key", "apikey",
            "credential", "cred", "login",
            "session", "cookie", "jwt",
            "private", "signature", "hash",
            "cert", "certificate", "pem"
        ];
        
        sensitive_patterns.iter().any(|pattern| key_lower.contains(pattern))
    }

    pub fn add_recovery_suggestion(mut self, suggestion: &str) -> Self {
        self.recovery_suggestions.push(suggestion.to_string());
        self
    }

    pub fn set_retryable(mut self, retryable: bool) -> Self {
        self.is_retryable = retryable;
        self
    }

    pub fn set_severity(mut self, severity: ErrorSeverity) -> Self {
        self.severity = severity;
        self
    }

    /// Format the error context as a detailed error message
    pub fn format_detailed(&self) -> String {
        let mut message = format!(
            "Error [{}] in {} during {}\n",
            self.error_id, self.component, self.operation
        );

        if !self.error_chain.is_empty() {
            message.push_str("Error Chain:\n");
            for (i, cause) in self.error_chain.iter().enumerate() {
                message.push_str(&format!("  {}: {}\n", i + 1, cause));
            }
        }

        if !self.context_data.is_empty() {
            message.push_str("Context:\n");
            for (key, value) in &self.context_data {
                message.push_str(&format!("  {key}: {value}\n"));
            }
        }

        if !self.recovery_suggestions.is_empty() {
            message.push_str("Recovery Suggestions:\n");
            for suggestion in &self.recovery_suggestions {
                message.push_str(&format!("  - {suggestion}\n"));
            }
        }

        message.push_str(&format!("Retryable: {}\n", self.is_retryable));
        message.push_str(&format!("Severity: {:?}\n", self.severity));

        message
    }
}

impl std::fmt::Display for ErrorContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} in {} ({})",
            self.operation, self.component, self.error_id
        )
    }
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Connection error: {0}")]
    Connection(String),

    #[error("MCP protocol error: {0}")]
    Mcp(String),

    #[error("BRP error: {0}")]
    Brp(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("UUID error: {0}")]
    Uuid(#[from] uuid::Error),

    #[error("Debug error: {0}")]
    DebugError(String),
    
    #[error("Checkpoint error: {0}")]
    Checkpoint(String),
    
    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Security error: {0}")]
    SecurityError(String),

    /// Rich error with full context
    #[error("Error: {context}")]
    WithContext {
        context: Box<ErrorContext>,
        #[source]
        source: Option<Box<Error>>,
    },
}
