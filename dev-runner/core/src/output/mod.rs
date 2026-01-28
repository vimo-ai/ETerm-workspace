//! Output module
//!
//! Defines output event types for process output streaming.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Type of output event
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputType {
    /// Build output (compilation, linking, etc.)
    BuildOutput,
    /// Application log
    Log,
    /// Error message
    Error,
    /// Process metrics update
    Metric,
    /// Process lifecycle event
    Lifecycle,
}

/// Log level
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    /// Parse log level from string
    pub fn from_str(s: &str) -> Self {
        let lower = s.to_lowercase();
        if lower.contains("error") || lower.contains("fatal") {
            LogLevel::Error
        } else if lower.contains("warn") {
            LogLevel::Warn
        } else if lower.contains("debug") || lower.contains("trace") {
            LogLevel::Debug
        } else {
            LogLevel::Info
        }
    }
}

/// An output event from a running process
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputEvent {
    /// Timestamp of the event
    pub timestamp: DateTime<Utc>,
    /// Type of event
    pub event_type: OutputType,
    /// Log level
    pub level: LogLevel,
    /// Content (format depends on adapter)
    pub content: String,
    /// Source stream (stdout or stderr)
    pub source: OutputSource,
}

/// Source of output
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputSource {
    Stdout,
    Stderr,
    System,
}

impl OutputEvent {
    /// Create a new output event
    pub fn new(event_type: OutputType, level: LogLevel, content: String) -> Self {
        Self {
            timestamp: Utc::now(),
            event_type,
            level,
            content,
            source: OutputSource::Stdout,
        }
    }

    /// Create a stdout event
    pub fn stdout(content: String) -> Self {
        Self {
            timestamp: Utc::now(),
            event_type: OutputType::Log,
            level: LogLevel::Info,
            content,
            source: OutputSource::Stdout,
        }
    }

    /// Create a stderr event
    pub fn stderr(content: String) -> Self {
        Self {
            timestamp: Utc::now(),
            event_type: OutputType::Error,
            level: LogLevel::Error,
            content,
            source: OutputSource::Stderr,
        }
    }

    /// Create a build output event
    pub fn build(content: String) -> Self {
        Self {
            timestamp: Utc::now(),
            event_type: OutputType::BuildOutput,
            level: LogLevel::Info,
            content,
            source: OutputSource::Stdout,
        }
    }

    /// Create a lifecycle event
    pub fn lifecycle(content: String) -> Self {
        Self {
            timestamp: Utc::now(),
            event_type: OutputType::Lifecycle,
            level: LogLevel::Info,
            content,
            source: OutputSource::System,
        }
    }

    /// Format for display
    pub fn format(&self) -> String {
        format!(
            "[{}] [{}] {}",
            self.timestamp.format("%H:%M:%S%.3f"),
            format!("{:?}", self.level).to_uppercase(),
            self.content
        )
    }

    /// Format for log file
    pub fn format_for_log(&self) -> String {
        format!(
            "[{}] [{:?}] {}",
            self.timestamp.to_rfc3339(),
            self.level,
            self.content
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_level_from_str() {
        assert_eq!(LogLevel::from_str("ERROR: something"), LogLevel::Error);
        assert_eq!(LogLevel::from_str("warning: check"), LogLevel::Warn);
        assert_eq!(LogLevel::from_str("DEBUG: trace"), LogLevel::Debug);
        assert_eq!(LogLevel::from_str("just info"), LogLevel::Info);
    }

    #[test]
    fn test_output_event_format() {
        let event = OutputEvent::stdout("Hello world".to_string());
        let formatted = event.format();
        assert!(formatted.contains("INFO"));
        assert!(formatted.contains("Hello world"));
    }
}
