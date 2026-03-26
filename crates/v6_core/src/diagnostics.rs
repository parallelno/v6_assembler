use std::fmt;

/// Source location for error reporting
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocation {
    pub file: String,
    pub line: usize,
    pub col: usize,
}

impl fmt::Display for SourceLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.file, self.line, self.col)
    }
}

/// Assembler error types
#[derive(Debug, Clone)]
pub struct AsmError {
    pub location: Option<SourceLocation>,
    pub message: String,
    pub source_line: Option<String>,
}

impl fmt::Display for AsmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(loc) = &self.location {
            write!(f, "{}: error: {}", loc, self.message)?;
            if let Some(line) = &self.source_line {
                write!(f, "\n  {}", line)?;
                if loc.col > 0 {
                    write!(f, "\n  {}^", " ".repeat(loc.col.saturating_sub(1)))?;
                }
            }
        } else {
            write!(f, "error: {}", self.message)?;
        }
        Ok(())
    }
}

impl std::error::Error for AsmError {}

impl AsmError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            location: None,
            message: message.into(),
            source_line: None,
        }
    }

    pub fn with_location(mut self, loc: SourceLocation) -> Self {
        self.location = Some(loc);
        self
    }

    pub fn with_source_line(mut self, line: impl Into<String>) -> Self {
        self.source_line = Some(line.into());
        self
    }
}

pub type AsmResult<T> = Result<T, AsmError>;
