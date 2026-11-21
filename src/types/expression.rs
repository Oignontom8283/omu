//! Mathematical expressions typed in a block, read by the `fasteval` library.
//!
//! A block that has one (the "expression" and the "condition" blocks) has as many inputs as its author asks for, named `a`, `b`, `c`...
//! (up to [`MAX_EXPRESSION_ARGS`]), and the expression uses them as variables: `a + b`, `c * log(a)`, `min(a, b) / 2`. A comparison
//! (`a = b`, `a != b`, `a >= 3`) gives 1 when it is true and 0 when it is not, and `and` / `or` (or `&&` / `||`) join comparisons, so that
//! `a >= 3 and b < 10` is a condition. `fasteval` brings the operators (`+ - * / % ^`), the comparisons, and the functions: `log`
//! (base 10, or `log(base, x)`), `min`, `max`, `abs`, `round`, `int`, `ceil`, `floor`, `sign`, `e()`, `pi()`, `sin`... (see [`FUNCTIONS`]).
//!
//! One thing is added to what `fasteval` reads: a single `=` is an equality too (`a = b`), as people write it; see [`prepare`].
//!
//! An expression is read once, when the plan is made ([`Compiled::new`], which also finds the mistakes: a word that is not an input, a
//! missing parenthesis), then calculated at each cycle of the graph by [`Compiled::eval`], which does not allocate.

use fasteval::{Compiler, Error, Evaler, Instruction, Parser, Slab};

/// The most inputs an expression block can have (`a` to `p`).
pub const MAX_EXPRESSION_ARGS: u32 = 16;
/// The longest expression.
pub const MAX_EXPRESSION_LEN: usize = 512;

/// The name of the `k`th input (from 0): `a`, `b`, `c`...
pub fn variable_name(k: usize) -> char {
    char::from(b'a' + (k.min(MAX_EXPRESSION_ARGS as usize - 1)) as u8)
}

/// The functions an expression can call, with how they are written and what they give (the interface completes them from this list).
pub const FUNCTIONS: &[(&str, &str)] = &[
    ("abs(x)", "the number without its sign"),
    ("ceil(x)", "rounded up"),
    ("floor(x)", "rounded down"),
    ("int(x)", "without its decimal part"),
    ("round(x)", "rounded to the nearest whole number"),
    ("sign(x)", "-1, 0 or 1"),
    ("min(x, y)", "the smaller (more numbers can be given)"),
    ("max(x, y)", "the greater (more numbers can be given)"),
    (
        "log(x)",
        "logarithm in base 10; log(base, x) for another base",
    ),
    ("e()", "the number e"),
    ("pi()", "the number pi"),
    ("sin(x)", "sine (in radians)"),
    ("cos(x)", "cosine (in radians)"),
    ("tan(x)", "tangent (in radians)"),
    ("asin(x)", "arc sine"),
    ("acos(x)", "arc cosine"),
    ("atan(x)", "arc tangent"),
];

/// The expression as `fasteval` reads it: a single `=` becomes `==`. What is already one (`==`, `<=`, `>=`, `!=`) is left as it is.
pub fn prepare(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() + 2);
    for (i, &c) in chars.iter().enumerate() {
        if c != '=' {
            out.push(c);
            continue;
        }
        let joined = i
            .checked_sub(1)
            .is_some_and(|j| matches!(chars[j], '<' | '>' | '!' | '='))
            || chars.get(i + 1) == Some(&'=');
        out.push_str(if joined { "=" } else { "==" });
    }
    out
}

/// An expression read and ready to be calculated.
pub struct Compiled {
    slab: Slab,
    instruction: Instruction,
    args: usize,
}

impl std::fmt::Debug for Compiled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Compiled")
            .field("args", &self.args)
            .finish_non_exhaustive()
    }
}

/// What is wrong with an expression, in words for the person who typed it.
fn describe(error: &Error, args: usize) -> String {
    match error {
        Error::Undefined(name) => {
            let inputs: Vec<String> = (0..args).map(|k| variable_name(k).to_string()).collect();
            format!(
                "\"{name}\" is not an input of this block (its inputs are {})",
                inputs.join(", ")
            )
        }
        Error::EOF | Error::EofWhileParsing(_) => {
            "the expression stops too soon (a parenthesis or an operand is missing)".into()
        }
        other => format!("the expression cannot be read: {other}"),
    }
}

impl Compiled {
    /// Reads `expression` for a block with `args` inputs; says what is wrong with it if it cannot be calculated.
    pub fn new(expression: &str, args: u32) -> Result<Compiled, String> {
        let args = args.clamp(1, MAX_EXPRESSION_ARGS) as usize;
        if expression.trim().is_empty() {
            return Err("write an expression, like a + b".into());
        }
        if expression.chars().count() > MAX_EXPRESSION_LEN {
            return Err(format!(
                "the expression is longer than {MAX_EXPRESSION_LEN} characters"
            ));
        }
        // `print` writes to the screen at each cycle: it is not a calculation
        if expression.contains("print") {
            return Err("print is not available in an expression".into());
        }
        let mut slab = Slab::new();
        let instruction = Parser::new()
            .parse(&prepare(expression), &mut slab.ps)
            .map_err(|e| describe(&e, args))?
            .from(&slab.ps)
            .compile(&slab.ps, &mut slab.cs);
        let compiled = Compiled {
            slab,
            instruction,
            args,
        };
        // calculated once with every input at 1: a word that is not an input is found now, not at the first cycle
        compiled
            .try_eval(&vec![1.0; args])
            .map_err(|e| describe(&e, args))?;
        Ok(compiled)
    }

    fn try_eval(&self, values: &[f64]) -> Result<f64, Error> {
        let args = self.args;
        let mut inputs = |name: &str, given: Vec<f64>| -> Option<f64> {
            let mut chars = name.chars();
            match (chars.next(), chars.next(), given.is_empty()) {
                (Some(c), None, true)
                    if ('a'..='p').contains(&c) && (c as usize - 'a' as usize) < args =>
                {
                    Some(
                        values
                            .get(c as usize - 'a' as usize)
                            .copied()
                            .unwrap_or(0.0),
                    )
                }
                _ => None,
            }
        };
        self.instruction.eval(&self.slab, &mut inputs)
    }

    /// The value of the expression for these inputs (`a`, `b`, `c`... in order). A result that is not a number (a division of 0 by 0,
    /// a logarithm of a negative number) or that cannot be calculated is 0: a block of the graph never sends what is not a number on.
    pub fn eval(&self, values: &[f64]) -> f64 {
        match self.try_eval(values) {
            Ok(v) if v.is_finite() => v,
            _ => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(expression: &str, values: &[f64]) -> f64 {
        Compiled::new(expression, values.len() as u32)
            .unwrap_or_else(|e| panic!("{expression}: {e}"))
            .eval(values)
    }

    #[test]
    fn arithmetic_and_functions_with_the_inputs_as_variables() {
        assert_eq!(eval("a + b", &[2.0, 3.0]), 5.0);
        assert_eq!(eval("a + b * 2", &[2.0, 3.0]), 8.0);
        assert!(
            (eval("c * log(a)", &[100.0, 0.0, 5.0]) - 10.0).abs() < 1e-9,
            "log is base 10"
        );
        assert!(
            (eval("log(2, a)", &[8.0]) - 3.0).abs() < 1e-9,
            "log(base, x)"
        );
        assert_eq!(eval("min(a, b) / 2", &[8.0, 6.0]), 3.0);
        assert_eq!(eval("(a + 1) ^ 2", &[2.0]), 9.0);
        assert_eq!(eval("abs(a) + round(b)", &[-2.0, 2.6]), 5.0);
    }

    #[test]
    fn comparisons_give_one_or_zero_and_a_single_equals_sign_is_an_equality() {
        assert_eq!(eval("a = b", &[3.0, 3.0]), 1.0);
        assert_eq!(eval("a = b", &[3.0, 4.0]), 0.0);
        assert_eq!(eval("a == b", &[3.0, 3.0]), 1.0, "the usual way works too");
        assert_eq!(eval("a != b", &[3.0, 4.0]), 1.0);
        assert_eq!(eval("a >= b", &[4.0, 4.0]), 1.0);
        assert_eq!(eval("a <= b", &[5.0, 4.0]), 0.0);
        assert_eq!(eval("a > 3 and b < 10", &[4.0, 9.0]), 1.0);
        assert_eq!(eval("a > 3 and b < 10", &[4.0, 11.0]), 0.0);
        assert_eq!(eval("a > 3 or b < 10", &[1.0, 11.0]), 0.0);
        assert_eq!(eval("a > 3 || b < 10", &[1.0, 9.0]), 1.0);
        assert_eq!(prepare("a = b"), "a == b");
        assert_eq!(
            prepare("a <= b and c >= d and e != f and g == h"),
            "a <= b and c >= d and e != f and g == h"
        );
    }

    #[test]
    fn what_is_not_a_number_is_zero_never_sent_on() {
        assert_eq!(eval("a / b", &[1.0, 0.0]), 0.0);
        assert!(eval("a / b", &[0.0, 0.0]).is_finite());
        assert_eq!(eval("log(a)", &[-1.0]), 0.0);
    }

    #[test]
    fn a_mistake_is_said_in_words_and_found_before_anything_runs() {
        let say = |expression: &str, args: u32| Compiled::new(expression, args).unwrap_err();
        assert!(say("", 2).contains("write an expression"));
        assert!(say("   ", 2).contains("write an expression"));
        let unknown = say("a + z", 2);
        assert!(
            unknown.contains("\"z\"") && unknown.contains("a, b"),
            "{unknown}"
        );
        let beyond = say("a + c", 2);
        assert!(
            beyond.contains("\"c\""),
            "an input the block does not have: {beyond}"
        );
        assert!(say("print(a)", 2).contains("not available"));
        assert!(say("a +", 2).len() > 5);
        assert!(say("(a + b", 2).len() > 5);
        assert!(say(&"a+".repeat(300), 2).contains("longer than"));
        assert_eq!(variable_name(0), 'a');
        assert_eq!(variable_name(15), 'p');
    }
}
