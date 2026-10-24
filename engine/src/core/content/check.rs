use std::fmt::Display;

use super::report::ContentProblem;

#[derive(Default)]
pub struct RowCheck {
    errors: Vec<RowError>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowError {
    pub pointer: String,
    pub problem: ContentProblem,
}

impl RowCheck {
    pub fn error(
        &mut self,
        pointer: impl Into<String>,
        rule: &'static str,
        found: impl Display,
        expected: impl Display,
    ) {
        self.problem(pointer, ContentProblem::new(rule, found, expected));
    }

    pub fn problem(&mut self, pointer: impl Into<String>, problem: ContentProblem) {
        self.errors.push(RowError {
            pointer: pointer.into(),
            problem,
        });
    }

    pub fn range<T: PartialOrd + Display + Copy>(
        &mut self,
        pointer: impl Into<String>,
        value: T,
        min: T,
        max: T,
        what: &str,
    ) {
        if value < min || value > max {
            self.error(
                pointer,
                "out of range",
                value,
                format!("{what} {min} to {max}"),
            );
        }
    }

    pub fn text(&mut self, pointer: impl Into<String>, text: &str, min: usize, max: usize) {
        let length = text.chars().count();
        let blank = min > 0 && text.trim().is_empty();
        if blank || length < min || length > max {
            let expected = match min {
                0 => format!("at most {max} characters"),
                _ => format!("{min} to {max} characters, not blank"),
            };
            self.error(pointer, "out of range", format!("{text:?}"), expected);
        }
    }

    pub fn count(
        &mut self,
        pointer: impl Into<String>,
        count: usize,
        min: usize,
        max: usize,
        what: &str,
    ) {
        if count < min || count > max {
            self.error(
                pointer,
                "out of range",
                format!("{count} {what}"),
                format!("{min} to {max} {what}"),
            );
        }
    }

    pub fn extend(&mut self, other: RowCheck) {
        self.errors.extend(other.errors);
    }

    pub fn into_errors(self) -> Vec<RowError> {
        self.errors
    }

    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }
}
