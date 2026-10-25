use std::collections::BTreeMap;
use std::fmt::{self, Display};
use ui::words;

use crate::core::math::Pos;
use crate::core::units::Meters;

#[derive(Clone, Debug, PartialEq)]
pub struct LoadError {
    pub file: String,
    pub at: ErrorPlace,
    pub problem: ContentProblem,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContentProblem {
    pub rule: &'static str,
    pub found: String,
    pub expected: String,
    pub nearest: Option<String>,
    /// A point of the ground where what is wrong would be fine, when the rule knows one.
    pub spot: Option<Pos<Meters>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorPlace {
    WholeFile,
    Json {
        line: usize,
        column: usize,
        pointer: String,
    },
    Csv {
        row: usize,
        column: usize,
    },
    Text {
        line: usize,
        column: usize,
    },
}

pub const VERSION_RULE: &str = "content version";

/// The rule a path breaks when nothing is at it.
pub const MISSING_FILE: &str = "missing file";

pub struct LoadReport {
    pub folder: String,
    pub build: String,
    pub version: String,
    pub outcome: String,
    pub errors: Vec<LoadError>,
    /// What would have loaded with a cost named, had nothing been refused.
    pub warnings: Vec<LoadError>,
}

impl LoadReport {
    pub fn new(
        folder: impl Into<String>,
        build: impl Into<String>,
        version: impl Into<String>,
        outcome: impl Into<String>,
        mut errors: Vec<LoadError>,
    ) -> LoadReport {
        errors.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
        LoadReport {
            folder: folder.into(),
            build: build.into(),
            version: version.into(),
            outcome: outcome.into(),
            errors,
            warnings: Vec::new(),
        }
    }

    pub fn with_warnings(mut self, mut warnings: Vec<LoadError>) -> LoadReport {
        warnings.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
        self.warnings = warnings;
        self
    }

    pub fn files(&self) -> usize {
        let mut files: Vec<&str> = self
            .errors
            .iter()
            .map(|error| error.file.as_str())
            .collect();
        files.sort_unstable();
        files.dedup();
        files.len()
    }
}

impl Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}: {}", self.at.location(&self.file), self.problem)
    }
}

impl ContentProblem {
    pub fn new(rule: &'static str, found: impl Display, expected: impl Display) -> ContentProblem {
        ContentProblem {
            rule,
            found: found.to_string(),
            expected: expected.to_string(),
            nearest: None,
            spot: None,
        }
    }

    pub fn near(self, nearest: Option<String>) -> ContentProblem {
        ContentProblem { nearest, ..self }
    }

    pub fn at_spot(self, spot: Option<Pos<Meters>>) -> ContentProblem {
        ContentProblem { spot, ..self }
    }

    pub fn unknown_row(table: &str, path: &str, nearest: Option<&str>) -> ContentProblem {
        ContentProblem::new(
            MISSING_FILE,
            format!("{path:?}"),
            format!("the path of {} {table} file", article(table)),
        )
        .near(nearest.map(|near| format!("{near:?}")))
    }
}

impl Display for ContentProblem {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}: {}; expected {}",
            self.rule, self.found, self.expected
        )?;
        if let Some(nearest) = &self.nearest {
            write!(f, " (nearest {nearest})")?;
        }
        Ok(())
    }
}

impl ErrorPlace {
    pub fn location(&self, file: &str) -> String {
        match self {
            ErrorPlace::WholeFile => file.to_owned(),
            ErrorPlace::Json { line, column, .. }
            | ErrorPlace::Csv { row: line, column }
            | ErrorPlace::Text { line, column } => format!("{file}:{line}:{column}"),
        }
    }

    /// The JSON pointer to where the problem is, the whole document's when it has no finer place.
    pub fn pointer(&self) -> &str {
        match self {
            ErrorPlace::Json { pointer, .. } => pointer,
            _ => "",
        }
    }

    pub fn within(&self) -> String {
        match self {
            ErrorPlace::WholeFile => "whole file".to_owned(),
            ErrorPlace::Json { pointer, .. } => pointer.clone(),
            ErrorPlace::Csv { row, column } => format!("row {row}, column {column}"),
            ErrorPlace::Text { line, column } => format!("line {line}, column {column}"),
        }
    }
}

type SortKey<'a> = (u8, &'a str, usize, usize, String, &'a str);

fn sort_key(error: &LoadError) -> SortKey<'_> {
    let (line, column, within) = match &error.at {
        ErrorPlace::WholeFile => (0, 0, String::new()),
        ErrorPlace::Json {
            line,
            column,
            pointer,
        } => (*line, *column, pointer.clone()),
        ErrorPlace::Csv { row, column } | ErrorPlace::Text { line: row, column } => {
            (*row, *column, String::new())
        }
    };
    let rank = match (
        error.problem.rule == VERSION_RULE,
        error.file == super::SETTINGS_FILE,
    ) {
        (true, _) => 2,
        (false, true) => 0,
        (false, false) => 1,
    };
    (rank, &error.file, line, column, within, error.problem.rule)
}

impl Display for LoadReport {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "# Rift content load report")?;
        writeln!(f, "folder   {}", self.folder)?;
        writeln!(
            f,
            "engine   rift {} · reads content version {}",
            self.build, self.version
        )?;
        writeln!(
            f,
            "result   not loaded · {} · {}",
            errors_in_files(self.errors.len(), self.files()),
            self.outcome
        )?;
        writeln!(f)?;
        writeln!(f, "## Errors by rule")?;
        let mut by_rule: BTreeMap<&str, usize> = BTreeMap::new();
        for error in &self.errors {
            *by_rule.entry(error.problem.rule).or_default() += 1;
        }
        let mut rules: Vec<(&str, usize)> = by_rule.into_iter().collect();
        rules.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        for line in rules.chunks(3) {
            let cells: Vec<String> = line
                .iter()
                .map(|(rule, count)| format!("{rule}  {count}"))
                .collect();
            writeln!(f, "{}", cells.join("    "))?;
        }
        writeln!(f)?;
        writeln!(f, "## Errors, by file and line")?;
        let mut index = 0;
        while index < self.errors.len() {
            let error = &self.errors[index];
            let mut same: Vec<String> = Vec::new();
            let mut next = index + 1;
            while next < self.errors.len() && repeats(error, &self.errors[next]) {
                same.push(self.errors[next].at.location(&self.errors[next].file));
                next += 1;
            }
            writeln!(
                f,
                "{}  {}",
                error.at.location(&error.file),
                error.at.within()
            )?;
            writeln!(f, "  rule      {}", error.problem.rule)?;
            writeln!(f, "  found     {}", error.problem.found)?;
            writeln!(f, "  expected  {}", error.problem.expected)?;
            if let Some(nearest) = &error.problem.nearest {
                writeln!(f, "  nearest   {nearest}")?;
            }
            if !same.is_empty() {
                writeln!(f, "  same in   {} more: {}", same.len(), same.join(", "))?;
            }
            index = next;
        }
        if !self.warnings.is_empty() {
            writeln!(f)?;
            writeln!(f, "## Warnings")?;
            for warning in &self.warnings {
                writeln!(f, "{}", warning)?;
            }
        }
        writeln!(f)?;
        write!(f, "## End · {}", words::counted(self.errors.len(), "error"))
    }
}

fn repeats(a: &LoadError, b: &LoadError) -> bool {
    a.at.within() == b.at.within()
        && a.problem.rule == b.problem.rule
        && a.problem.found == b.problem.found
        && a.problem.expected == b.problem.expected
        && a.file != b.file
}

fn errors_in_files(errors: usize, files: usize) -> String {
    format!(
        "{} in {}",
        words::counted(errors, "error"),
        words::counted(files, "file")
    )
}

pub(super) fn article(word: &str) -> &'static str {
    match word.chars().next() {
        Some('a' | 'e' | 'i' | 'o' | 'u') => "an",
        _ => "a",
    }
}
