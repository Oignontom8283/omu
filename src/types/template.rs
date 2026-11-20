//! The text templates of the `join` blocks: `Hello $arg1, you have ${arg2:0} messages \\$ not a value`.
//!
//! The substitution itself is not written here: it is the `subst` library's (shell-like variables, default values with
//! `${name:default}`, a backslash to write a `$`), so that the escaping rules are those of a known and tested tool. The
//! values of a block are `arg1`, `arg2`... (also `1`, `2`...).

use std::fmt;

/// What was wrong with a template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateError(pub String);

impl fmt::Display for TemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The values of a join block, by the names a template can use for them.
struct Values<'a>(&'a [String]);

impl<'a> subst::VariableMap<'a> for Values<'a> {
    type Value = &'a str;

    fn get(&'a self, name: &str) -> Option<&'a str> {
        let n: usize = name.strip_prefix("arg").unwrap_or(name).parse().ok()?;
        self.0.get(n.checked_sub(1)?).map(String::as_str)
    }
}

/// Makes the text of `template` from the written values of the block (`values[0]` is `arg1`).
pub fn render(template: &str, values: &[String]) -> Result<String, TemplateError> {
    subst::substitute(template, &Values(values)).map_err(|e| TemplateError(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn said(template: &str) -> Result<String, TemplateError> {
        render(
            template,
            &["Bob".to_owned(), "3".to_owned(), "1.5".to_owned()],
        )
    }

    #[test]
    fn values_go_where_the_dollar_signs_are() {
        assert_eq!(said("hi $arg1").unwrap(), "hi Bob");
        assert_eq!(said("hi $1").unwrap(), "hi Bob");
        assert_eq!(
            said("${arg1} has ${arg2} items").unwrap(),
            "Bob has 3 items"
        );
        assert_eq!(said("$arg1$arg2").unwrap(), "Bob3");
        assert_eq!(said("no values at all").unwrap(), "no values at all");
    }

    #[test]
    fn a_default_is_used_for_a_value_that_does_not_exist_and_a_backslash_writes_a_dollar() {
        assert_eq!(said("${arg9:nobody}").unwrap(), "nobody");
        assert_eq!(said(r"cost \$5").unwrap(), "cost $5");
    }

    #[test]
    fn a_template_that_is_not_valid_says_why() {
        assert!(
            said("$arg9").unwrap_err().0.contains("arg9"),
            "a value that does not exist is named"
        );
        assert!(said("${arg1").is_err(), "a brace that is not closed");
        assert!(said("price: $").is_err(), "a dollar sign with no name");
        assert!(said(r"\q").is_err(), "an escape that does not exist");
    }
}
