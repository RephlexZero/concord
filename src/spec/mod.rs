//! The YAML spec: parsing ([`raw`]), and expansion of templates and tag sets
//! into the flat [`Desired`](crate::model::Desired) model ([`expand`]).

pub mod expand;
pub mod raw;

use std::path::Path;

pub use expand::expand;
pub use raw::SpecFile;

use crate::diag::Diagnostic;

#[derive(Debug, thiserror::Error)]
pub enum SpecError {
    #[error("can't read {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("{0}")]
    Parse(Diagnostic),
}

/// Parse spec text. Errors name the path of the bad value, e.g.
/// `categories.vd.channels[1].layout`.
pub fn parse(text: &str) -> Result<SpecFile, Diagnostic> {
    if text.trim().is_empty() {
        return Err(Diagnostic::error("", "the spec file is empty"));
    }
    let de = serde_yaml_ng::Deserializer::from_str(text);
    serde_path_to_error::deserialize(de).map_err(|err| {
        let path = err.path().to_string();
        let path = if path == "." { String::new() } else { path };
        let message = strip_yaml_path(err.into_inner().to_string());
        let hint = unknown_field_hint(&message);
        Diagnostic::error(path, message).with_hint_opt(hint)
    })
}

pub fn load(path: &Path) -> Result<SpecFile, SpecError> {
    let text = std::fs::read_to_string(path).map_err(|source| SpecError::Io {
        path: path.display().to_string(),
        source,
    })?;
    parse(&text).map_err(SpecError::Parse)
}

/// serde_yaml_ng prefixes messages with its own (less precise) path; we
/// already report the path separately.
fn strip_yaml_path(message: String) -> String {
    match message.split_once(": ") {
        Some((head, tail)) if !head.contains(' ') && !head.is_empty() => tail.to_owned(),
        _ => message,
    }
}

fn unknown_field_hint(msg: &str) -> Option<String> {
    let rest = msg.strip_prefix("unknown field `")?;
    let (field, rest) = rest.split_once('`')?;
    let expected = rest.split_once("expected ")?.1;
    let candidates: Vec<&str> = expected
        .split('`')
        .enumerate()
        .filter(|(i, _)| i % 2 == 1)
        .map(|(_, s)| s)
        .collect();
    crate::diag::suggest(field, candidates.iter().copied())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_errors_name_the_path() {
        let err = parse(
            "categories:\n  vd:\n    name: vd\n    channels:\n      - { key: a, type: text, name: a }\n      - { key: b, type: forum, name: b, layout: grid }\n",
        )
        .unwrap_err();
        assert_eq!(err.path, "categories.vd.channels[1].layout");
        assert!(err.message.contains("grid"), "{}", err.message);
    }

    #[test]
    fn unknown_fields_are_rejected_with_suggestion() {
        let err = parse("roles:\n  - { key: lead, name: Lead, colour: \"#fff\" }\n").unwrap_err();
        assert_eq!(err.path, "roles[0].colour");
        assert_eq!(
            err.hint.as_deref(),
            Some("did you mean `color`?"),
            "{}",
            err.message
        );
    }
}
