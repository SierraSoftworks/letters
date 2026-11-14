//! The command table against the engine it describes, and against what people type to find a symbol.

mod common;

use std::collections::HashSet;

use common::*;
use letters_math::{Command, commands, environments, typeset};

/// Whether a row answers to `query` as a completion list would match it: by command, name or alias.
fn answers(row: &Command, query: &str) -> bool {
    row.command.contains(query) || row.name.contains(query) || row.aliases.iter().any(|alias| alias.contains(query))
}

#[test]
fn every_row_typesets_and_draws_something() {
    for row in commands().iter().chain(environments()) {
        let formula = typeset(row.sample, &inline(16.0)).unwrap_or_else(|error| panic!("{}: {error}", row.command));

        assert!(!formula.items.is_empty(), "{} draws nothing", row.command);
        assert!(row.sample.contains(row.command), "the sample for {} does not use it", row.command);
    }
}

#[test]
fn rows_are_distinct_and_described() {
    for (table, prefix) in [(commands(), "\\"), (environments(), "")] {
        let mut seen = HashSet::new();
        for row in table {
            assert!(seen.insert(row.command), "{} is listed twice", row.command);
            assert!(row.command.starts_with(prefix) && !row.name.is_empty(), "{row:?}");
        }
    }
}

#[test]
fn symbols_are_found_by_what_people_call_them() {
    // (what is typed, a command it must find)
    let cases = [
        ("arrow", r"\rightarrow"),
        ("not equal", r"\neq"),
        ("!=", r"\neq"),
        ("<=", r"\leq"),
        ("->", r"\to"),
        ("implies", r"\Rightarrow"),
        ("fraction", r"\frac"),
        ("root", r"\sqrt"),
        ("infinity", r"\infty"),
        ("integral", r"\int"),
        ("sum", r"\sum"),
        ("alpha", r"\alpha"),
        ("greek", r"\omega"),
        ("for all", r"\forall"),
        ("element of", r"\in"),
        ("empty set", r"\emptyset"),
        ("multiply", r"\times"),
        ("gradient", r"\nabla"),
        ("vector", r"\vec"),
        ("blackboard", r"\mathbb"),
        ("bold", r"\mathbf"),
        ("colour", r"\color"),
        ("color", r"\textcolor"),
        ("strike", r"\cancel"),
        ("space", r"\quad"),
        ("absolute value", r"\lvert"),
        ("chemistry", r"\ce"),
        (r"\fra", r"\frac"),
    ];
    for (query, command) in cases {
        let found: Vec<_> = commands().iter().filter(|row| answers(row, query)).map(|row| row.command).collect();

        assert!(found.contains(&command), "{query:?} finds {found:?}, not {command}");
    }
}

#[test]
fn environments_are_found_by_what_people_call_them() {
    for (query, environment) in
        [("matrix", "pmatrix"), ("piecewise", "cases"), ("align", "aligned"), ("table", "array")]
    {
        let found: Vec<_> = environments().iter().filter(|row| answers(row, query)).map(|row| row.command).collect();

        assert!(found.contains(&environment), "{query:?} finds {found:?}, not {environment}");
    }
}

#[test]
fn arguments_count_the_braces_to_insert() {
    let counts =
        [(r"\alpha", 0), (r"\frac", 2), (r"\sqrt", 1), (r"\text", 1), (r"\left", 0), (r"\textcolor", 2), (r"\quad", 0)];
    for (command, arguments) in counts {
        let row = commands().iter().find(|row| row.command == command).unwrap_or_else(|| panic!("no {command}"));

        assert_eq!(row.arguments, arguments, "{command}");
    }
    // The inserted form is the start of something valid: filled in, it typesets.
    for row in commands().iter().filter(|row| row.arguments > 0) {
        let braces = row.sample.matches('{').count();
        assert!(
            braces >= usize::from(row.arguments),
            "{} takes {} but its sample fills {braces}",
            row.command,
            row.arguments
        );
    }
}

#[test]
fn the_everyday_commands_lead_the_list() {
    let lead: Vec<_> = commands().iter().take(60).map(|row| row.command).collect();

    for command in [r"\frac", r"\sqrt", r"\sum", r"\int", r"\alpha"] {
        assert!(lead.contains(&command), "{command} is not near the top");
    }
}
