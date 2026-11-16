//! Product branding, resolved at compile time from `branding.toml` (see that file).
//!
//! Never write the product's name as a literal anywhere else: use these constants, and the `{product}`
//! placeholder in user-facing texts ([`format_product`]).

include!(concat!(env!("OUT_DIR"), "/branding.rs"));

/// Replaces every `{product}` in `template` with [`DISPLAY_NAME`].
pub fn format_product(template: &str) -> String {
    template.replace("{product}", DISPLAY_NAME)
}

/// Name of an environment variable of the product: `env_var("LOG")` is `VL_LOG` with the default branding.
pub fn env_var(suffix: &str) -> String {
    format!("{ENV_PREFIX}_{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_are_not_empty() {
        for s in [
            DISPLAY_NAME,
            SLUG,
            CLI_EXECUTABLE,
            DAEMON_EXECUTABLE,
            APP_ID,
            DATA_DIR_NAME,
            ENV_PREFIX,
        ] {
            assert!(!s.is_empty());
        }
    }

    #[test]
    fn placeholder_is_replaced_everywhere() {
        let s = format_product("{product} is running; stop {product}?");
        assert!(!s.contains("{product}"));
        assert_eq!(s.matches(DISPLAY_NAME).count(), 2);
    }

    #[test]
    fn env_var_uses_the_prefix() {
        assert_eq!(env_var("LOG"), format!("{ENV_PREFIX}_LOG"));
    }
}
