//! Tests on texts typed in a block, read by the `cel` library (the Common Expression Language).
//!
//! The "text test" block has as many text inputs as its author asks for, named `a`, `b`, `c`... (up to [`MAX_TEXT_ARGS`]), and a test
//! written with them that says yes or no: `a == b`, `a.contains(b)`, `a.startsWith("hello")`, `a.matches("^[0-9]+$")` (a regular
//! expression), `size(a) > 3`, `a.lowerAscii() == b.lowerAscii()` (the case does not count), `a.contains(b) && !c.endsWith(".mp3")`.
//! Everything the library brings to texts is there: see [`EXAMPLES`] for the ones the interface shows.
//!
//! Two things are added to what the library reads, so that a test can be written the way people say it: a single `=` is an equality
//! (`a = b`), and `and` / `or` are `&&` / `||` ([`prepare`]; what is between quotes is never touched).
//!
//! A test is read once, when the plan is made ([`Compiled::new`], which also finds the mistakes: a word that is not an input, a missing
//! parenthesis, a result that is not yes or no), then asked again only when a text changed ([`Compiled::eval`]).

use cel::{Context, Env, ExecutionError, Program, Value};
use std::sync::Arc;

/// The most inputs a text test can have (`a` to `p`).
pub const MAX_TEXT_ARGS: u32 = 16;
/// The longest test.
pub const MAX_TEXT_EXPRESSION_LEN: usize = 512;
/// The longest text a test is given: what is longer is cut (a regular expression is worked out over the whole of a text).
pub const MAX_TEXT_LEN: usize = 4096;

/// What can be written, with what it does: shown in the block's documentation and completed in its field (the same texts, in the
/// interface's own files: this is the list of the forms, the words are the interface's).
pub const EXAMPLES: &[&str] = &[
    "a == b",
    "a = b",
    "a != b",
    "a.contains(b)",
    "a.startsWith(b)",
    "a.endsWith(\".mp3\")",
    "a.matches(\"^[0-9]+$\")",
    "size(a) > 3",
    "a.lowerAscii() == b.lowerAscii()",
    "a.contains(b) && !c.contains(\"no\")",
    "a.contains(b) and !c.contains(b)",
    "a.trim() != \"\"",
    "a.matches(\"(?i)stream\")",
    "a.indexOf(b) >= 0",
    "a.substring(0, 3) == \"abc\"",
    "a.replace(\"x\", \"y\") == b",
    "a.split(\",\").size() > 2",
    "a.upperAscii() == \"YES\"",
];

/// The name of the `k`th input (from 0): `a`, `b`, `c`...
pub fn variable_name(k: usize) -> char {
    char::from(b'a' + (k.min(MAX_TEXT_ARGS as usize - 1)) as u8)
}

/// The test as the library reads it: a single `=` becomes `==`, `and` becomes `&&` and `or` becomes `||`. What is already one of
/// these (`==`, `<=`, `>=`, `!=`) is left as it is, and so is everything between quotes.
pub fn prepare(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() + 4);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' | '\'' => {
                // a text: copied as it is, up to its closing quote (a backslash gives the next character away)
                out.push(c);
                i += 1;
                while i < chars.len() {
                    out.push(chars[i]);
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 1;
                        out.push(chars[i]);
                    } else if chars[i] == c {
                        break;
                    }
                    i += 1;
                }
                i += 1;
            }
            '=' => {
                let joined = i
                    .checked_sub(1)
                    .is_some_and(|j| matches!(chars[j], '<' | '>' | '!' | '='))
                    || chars.get(i + 1) == Some(&'=');
                out.push_str(if joined { "=" } else { "==" });
                i += 1;
            }
            c if c.is_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                // a word after a dot is the name of a function (`a.and`), not the operator
                let after_dot = start > 0 && chars[start - 1] == '.';
                out.push_str(match word.as_str() {
                    "and" if !after_dot => "&&",
                    "or" if !after_dot => "||",
                    _ => &word,
                });
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// A test read and ready to be asked.
pub struct Compiled {
    env: Arc<Env>,
    program: Program,
    args: usize,
}

impl std::fmt::Debug for Compiled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Compiled")
            .field("args", &self.args)
            .finish_non_exhaustive()
    }
}

fn inputs(args: usize) -> String {
    (0..args)
        .map(|k| variable_name(k).to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

impl Compiled {
    /// Reads `expression` for a block with `args` inputs; says what is wrong with it if it cannot be asked.
    pub fn new(expression: &str, args: u32) -> Result<Compiled, String> {
        let args = args.clamp(1, MAX_TEXT_ARGS) as usize;
        if expression.trim().is_empty() {
            return Err("write a test, like a == b".into());
        }
        if expression.chars().count() > MAX_TEXT_EXPRESSION_LEN {
            return Err(format!(
                "the test is longer than {MAX_TEXT_EXPRESSION_LEN} characters"
            ));
        }
        // the standard functions, and the ones of the texts library (`lowerAscii`, `trim`, `replace`, `split`, `indexOf`...)
        let mut env = Env::stdlib();
        cel::extensions::strings(&mut env).map_err(|e| e.to_string())?;
        let env = Arc::new(env);
        let program = env
            .compile(&prepare(expression))
            .map_err(|e| format!("the test cannot be read: {}", first_line(&e.to_string())))?;
        for name in program.references().variables() {
            let known =
                name.len() == 1 && ('a'..='p').contains(&name.chars().next().unwrap_or('?'));
            if !known || (name.as_bytes()[0] - b'a') as usize >= args {
                return Err(format!(
                    "\"{name}\" is not an input of this block (its inputs are {})",
                    inputs(args)
                ));
            }
        }
        let compiled = Compiled { env, program, args };
        // asked once with empty texts: a function that does not exist, or a result that is not yes or no, is found now
        match compiled.run(&vec![""; args]) {
            Ok(Value::Bool(_)) => {}
            Ok(_) => return Err("a test must say yes or no (a comparison, like a == b)".into()),
            Err(
                e @ (ExecutionError::UndeclaredReference(_)
                | ExecutionError::UnsupportedBinaryOperator(..)),
            ) => {
                return Err(format!(
                    "the test cannot be asked: {}",
                    first_line(&e.to_string())
                ))
            }
            // what depends on the texts (a conversion, a regular expression that is not one) is told when it happens
            Err(_) => {}
        }
        Ok(compiled)
    }

    fn run(&self, values: &[&str]) -> Result<Value, ExecutionError> {
        let mut context = Context::with_env(self.env.clone());
        for (k, name) in (0..self.args).map(|k| (k, variable_name(k).to_string())) {
            let text: String = values
                .get(k)
                .copied()
                .unwrap_or("")
                .chars()
                .take(MAX_TEXT_LEN)
                .collect();
            context.add_variable_from_value(name, text);
        }
        self.program.execute(&context)
    }

    /// Whether the test is true for these texts (`a`, `b`, `c`... in order). What cannot be asked (a regular expression that is not
    /// one, a conversion that fails) is false: a block of the graph never stops for a text.
    pub fn eval(&self, values: &[&str]) -> bool {
        matches!(self.run(values), Ok(Value::Bool(true)))
    }

    /// Why a test could not be asked for these texts, when it could not (`None` when it could).
    pub fn problem(&self, values: &[&str]) -> Option<String> {
        self.run(values).err().map(|e| first_line(&e.to_string()))
    }
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or(text).trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(expression: &str, values: &[&str]) -> bool {
        Compiled::new(expression, values.len() as u32)
            .unwrap_or_else(|e| panic!("{expression}: {e}"))
            .eval(values)
    }

    #[test]
    fn texts_are_compared_and_searched() {
        assert!(eval("a == b", &["x", "x"]));
        assert!(!eval("a == b", &["x", "y"]));
        assert!(eval("a != b", &["x", "y"]));
        assert!(eval("a.contains(b)", &["hello world", "lo w"]));
        assert!(!eval("a.contains(b)", &["hello", "xyz"]));
        assert!(eval("a.startsWith(b)", &["hello", "he"]));
        assert!(eval("a.endsWith(\".mp3\")", &["song.mp3", ""]));
        assert!(eval("size(a) > 3", &["abcd"]));
        assert!(!eval("size(a) > 3", &["abc"]));
        assert!(eval(
            "a.lowerAscii() == b.lowerAscii()",
            &["Hello", "hELLO"]
        ));
        assert!(eval(
            "a.contains(b) && !c.contains(\"no\")",
            &["abc", "b", "yes"]
        ));
    }

    #[test]
    fn a_regular_expression_is_one_of_the_texts_functions() {
        assert!(eval("a.matches(\"^[0-9]+$\")", &["12345"]));
        assert!(!eval("a.matches(\"^[0-9]+$\")", &["12a45"]));
        assert!(eval("a.matches(b)", &["order-42", "[a-z]+-[0-9]+"]));
        // a pattern that is not one is false, and says why
        let bad = Compiled::new("a.matches(b)", 2).unwrap();
        assert!(!bad.eval(&["x", "("]));
        assert!(bad.problem(&["x", "("]).is_some());
        assert!(bad.problem(&["x", "x"]).is_none());
    }

    #[test]
    fn a_single_equals_sign_and_the_words_and_or_are_read_as_people_write_them() {
        assert!(eval("a = b", &["x", "x"]));
        assert!(!eval("a = b", &["x", "y"]));
        assert!(eval("a.contains(b) and a.contains(c)", &["abc", "a", "c"]));
        assert!(eval("a == \"x\" or b == \"y\"", &["z", "y"]));
        assert_eq!(prepare("a = b"), "a == b");
        assert_eq!(prepare("a != b and a <= c"), "a != b && a <= c");
        // what is between quotes is left alone
        assert_eq!(prepare("a == \"x = y and z\""), "a == \"x = y and z\"");
        assert_eq!(prepare("a == 'it\\'s = or'"), "a == 'it\\'s = or'");
        assert!(eval("a = \"x = y\"", &["x = y"]));
    }

    #[test]
    fn a_mistake_is_said_in_words_and_found_before_anything_runs() {
        let say = |expression: &str, args: u32| Compiled::new(expression, args).unwrap_err();
        assert!(say("", 2).contains("write a test"));
        let unknown = say("a == z", 2);
        assert!(
            unknown.contains("\"z\"") && unknown.contains("a, b"),
            "{unknown}"
        );
        assert!(
            say("a == c", 2).contains("\"c\""),
            "an input the block does not have"
        );
        assert!(say("a ==", 2).contains("cannot be read"));
        assert!(say("(a == b", 2).contains("cannot be read"));
        assert!(
            say("a + b", 2).contains("yes or no"),
            "a result that is not a yes or no"
        );
        assert!(
            say("a.nothing(b)", 2).contains("cannot be asked"),
            "{}",
            say("a.nothing(b)", 2)
        );
        assert!(say(&"a == b && ".repeat(100), 2).contains("longer than"));
        assert_eq!(variable_name(0), 'a');
        assert_eq!(variable_name(15), 'p');
    }

    #[test]
    fn the_examples_that_are_shown_all_work() {
        for example in EXAMPLES {
            Compiled::new(example, 3).unwrap_or_else(|e| panic!("{example}: {e}"));
        }
    }
}
