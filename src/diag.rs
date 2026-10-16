//! Diagnostics: validation and planning problems, each tied to a spec path.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    /// Where the problem is, e.g. `categories.vd.channels[1].tags`. Empty for
    /// problems that are not tied to one place in the spec.
    pub path: String,
    pub message: String,
    pub hint: Option<String>,
}

impl Diagnostic {
    pub fn error(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            path: path.into(),
            message: message.into(),
            hint: None,
        }
    }

    pub fn warning(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            path: path.into(),
            message: message.into(),
            hint: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn with_hint_opt(mut self, hint: Option<String>) -> Self {
        self.hint = hint;
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let level = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        if self.path.is_empty() {
            write!(f, "{level}: {}", self.message)?;
        } else {
            write!(f, "{level}: {}: {}", self.path, self.message)?;
        }
        if let Some(hint) = &self.hint {
            write!(f, "\n  hint: {hint}")?;
        }
        Ok(())
    }
}

pub fn has_errors(diags: &[Diagnostic]) -> bool {
    diags.iter().any(Diagnostic::is_error)
}

/// Suggest the closest candidate to `input` ("did you mean ...?").
pub fn suggest<'a>(input: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let input_lower = input.to_lowercase();
    let mut best: Option<(usize, &str)> = None;
    for candidate in candidates {
        let distance = levenshtein(&input_lower, &candidate.to_lowercase());
        if best.is_none_or(|(d, _)| distance < d) {
            best = Some((distance, candidate));
        }
    }
    let (distance, candidate) = best?;
    let max = (input.chars().count() / 3).max(2);
    (distance <= max).then(|| format!("did you mean `{candidate}`?"))
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_close_matches_only() {
        assert_eq!(
            suggest("Leda", ["Lead", "Director"]).as_deref(),
            Some("did you mean `Lead`?")
        );
        assert_eq!(suggest("Treasurer", ["Lead", "Director"]), None);
    }
}
