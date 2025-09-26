pub struct DjangoDockerfile {
    pub environment_vars: Vec<String>,
}

impl DjangoDockerfile {
    pub fn new() -> Self {
        Self {
            environment_vars: Vec::new(),
        }
    }
    
    pub fn with_environment(mut self, env_vars: Vec<String>) -> Self {
        self.environment_vars = env_vars;
        self
    }

    pub fn generate(&self) -> String {
        let mut dockerfile = String::from(r#"
# Multi-stage build for smaller image
FROM python:3.11-alpine AS builder

WORKDIR /app

# Install build dependencies
RUN apk add --no-cache gcc musl-dev

# Install Python packages
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

# Runtime stage
FROM python:3.11-alpine AS runtime

WORKDIR /app

# Copy Python packages from builder
COPY --from=builder /usr/local/lib/python3.11/site-packages /usr/local/lib/python3.11/site-packages
COPY --from=builder /usr/local/bin /usr/local/bin

# Copy app
COPY . .
"#);

        // Add environment variables
        if !self.environment_vars.is_empty() {
            dockerfile.push_str("\n# Environment variables\n");
            for env_var in &self.environment_vars {
                dockerfile.push_str(&format!("ENV {}\n", env_var));
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
