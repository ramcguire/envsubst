use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::convert::Infallible;
use std::env;

/// An environment variable lookup context for use with `shellexpand::env_with_context`.
///
/// When `use_real_env` is true (no --env-file given), variables are looked
/// up from the system and cached.
/// 
/// When `use_real_env` is false (--env-file provided), variables are looked
/// up in the provided file(s).
///
/// Missing variables are tracked with access counts.
pub struct EnvContext {
    found: RefCell<HashMap<String, String>>,
    missing: RefCell<HashMap<String, usize>>,
    preloaded: HashMap<String, String>,
    use_real_env: bool,
}

impl EnvContext {
    pub fn new(preloaded: HashMap<String, String>, use_real_env: bool) -> Self {
        Self {
            found: RefCell::new(HashMap::new()),
            missing: RefCell::new(HashMap::new()),
            preloaded,
            use_real_env,
        }
    }

    /// Look up a variable. Returns `Ok(Some(...))` if found, `Ok(None)` if missing.
    /// Never returns `Err` — `Infallible` reflects this.
    pub fn lookup(&self, key: &str) -> Result<Option<Cow<'static, str>>, Infallible> {
        // Cache hit: found
        if let Some(val) = self.found.borrow().get(key) {
            return Ok(Some(Cow::Owned(val.clone())));
        }

        // Cache hit: missing — increment count
        if let Some(count) = self.missing.borrow_mut().get_mut(key) {
            *count += 1;
            return Ok(None);
        }

        // First-time lookup
        let val = if self.use_real_env {
            env::var(key).ok()
        } else {
            self.preloaded.get(key).cloned()
        };

        match val {
            Some(v) => {
                self.found.borrow_mut().insert(key.to_string(), v.clone());
                Ok(Some(Cow::Owned(v)))
            }
            None => {
                self.missing.borrow_mut().insert(key.to_string(), 1);
                Ok(None)
            }
        }
    }

    pub fn missing_vars(&self) -> HashMap<String, usize> {
        self.missing.borrow().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_real() -> EnvContext {
        EnvContext::new(HashMap::new(), true)
    }

    fn ctx_file(pairs: &[(&str, &str)]) -> EnvContext {
        let map = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        EnvContext::new(map, false)
    }

    // Real env mode

    #[test]
    fn real_env_found() {
        temp_env::with_var("EV_FOUND", Some("hello"), || {
            assert_eq!(
                ctx_real().lookup("EV_FOUND").unwrap(),
                Some(Cow::Borrowed("hello"))
            );
        });
    }

    #[test]
    fn real_env_missing_returns_none() {
        temp_env::with_var("EV_ABSENT", None::<&str>, || {
            assert_eq!(ctx_real().lookup("EV_ABSENT").unwrap(), None);
        });
    }

    #[test]
    fn real_env_missing_tracked() {
        temp_env::with_var("EV_MISS", None::<&str>, || {
            let ctx = ctx_real();
            ctx.lookup("EV_MISS").unwrap();
            assert_eq!(ctx.missing_vars()["EV_MISS"], 1);
        });
    }

    #[test]
    fn real_env_missing_count_increments() {
        temp_env::with_var("EV_CNT", None::<&str>, || {
            let ctx = ctx_real();
            for _ in 0..3 {
                ctx.lookup("EV_CNT").unwrap();
            }
            assert_eq!(ctx.missing_vars()["EV_CNT"], 3);
        });
    }

    #[test]
    fn real_env_found_var_not_in_missing() {
        temp_env::with_var("EV_OK", Some("v"), || {
            let ctx = ctx_real();
            ctx.lookup("EV_OK").unwrap();
            assert!(ctx.missing_vars().is_empty());
        });
    }

    #[test]
    fn real_env_cache_survives_removal() {
        let ctx = ctx_real();
        temp_env::with_var("EV_CACHE", Some("cached"), || {
            ctx.lookup("EV_CACHE").unwrap();
        });
        temp_env::with_var("EV_CACHE", None::<&str>, || {
            assert_eq!(
                ctx.lookup("EV_CACHE").unwrap(),
                Some(Cow::Borrowed("cached"))
            );
        });
        assert!(ctx.missing_vars().is_empty());
    }

    // Env-file mode (use_real_env = false)

    #[test]
    fn file_mode_returns_preloaded_value() {
        temp_env::with_var("EV_FILE", None::<&str>, || {
            let ctx = ctx_file(&[("EV_FILE", "from-file")]);
            assert_eq!(
                ctx.lookup("EV_FILE").unwrap(),
                Some(Cow::Borrowed("from-file"))
            );
            assert!(ctx.missing_vars().is_empty());
        });
    }

    #[test]
    fn file_mode_ignores_real_env() {
        temp_env::with_var("EV_IGNORE", Some("real-value"), || {
            let ctx = ctx_file(&[("EV_IGNORE", "file-value")]);
            // Real env is ignored — file value wins
            assert_eq!(
                ctx.lookup("EV_IGNORE").unwrap(),
                Some(Cow::Borrowed("file-value"))
            );
        });
    }

    #[test]
    fn file_mode_missing_when_not_in_preloaded() {
        // Real env has the var, but file mode should not see it
        temp_env::with_var("EV_REAL_ONLY", Some("real"), || {
            let ctx = ctx_file(&[("OTHER", "irrelevant")]);
            assert_eq!(ctx.lookup("EV_REAL_ONLY").unwrap(), None);
            assert_eq!(ctx.missing_vars()["EV_REAL_ONLY"], 1);
        });
    }

    // Template integration

    #[test]
    fn template_substitutes_found_var() {
        temp_env::with_var("EV_T_FOUND", Some("world"), || {
            let ctx = ctx_real();
            let out =
                shellexpand::env_with_context("hello ${EV_T_FOUND}", |k| ctx.lookup(k)).unwrap();
            assert_eq!(out, "hello world");
        });
    }

    #[test]
    fn template_leaves_missing_as_literal() {
        temp_env::with_var("EV_T_MISS", None::<&str>, || {
            let ctx = ctx_real();
            let out =
                shellexpand::env_with_context("val: ${EV_T_MISS}", |k| ctx.lookup(k)).unwrap();
            assert_eq!(out, "val: ${EV_T_MISS}");
            assert_eq!(ctx.missing_vars()["EV_T_MISS"], 1);
        });
    }

    #[test]
    fn template_file_mode_substitutes_preloaded() {
        temp_env::with_var("EV_T_FILE", None::<&str>, || {
            let ctx = ctx_file(&[("EV_T_FILE", "from-dotenv")]);
            let out =
                shellexpand::env_with_context("cfg: ${EV_T_FILE}", |k| ctx.lookup(k)).unwrap();
            assert_eq!(out, "cfg: from-dotenv");
        });
    }
}
