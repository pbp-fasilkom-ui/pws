#!/bin/bash

echo "=== PBP Wildcard Certificate Generation ==="
echo "Domain: *.pbp.cs.ui.ac.id"
echo ""

CERT_DIR="/home/admin/pws/ssl/pbp"
mkdir -p "$CERT_DIR"

echo "Starting certificate generation..."
echo "You will need to ask admin to add TXT record when prompted."
echo ""

sudo certbot certonly --manual -d "*.pbp.cs.ui.ac.id" \
    --agree-tos --manual-public-ip-logging-ok \
    --preferred-challenges dns-01 \
    --email admin@pbp.cs.ui.ac.id \
    --no-eff-email \
    --work-dir "$CERT_DIR/work" \
    --config-dir "$CERT_DIR/config" \
    --logs-dir "$CERT_DIR/logs"

if [ $? -eq 0 ]; then
    echo ""
    echo "✅ Certificate generated successfully!"
    echo "Certificate location: $CERT_DIR/config/live/pbp.cs.ui.ac.id/"
    echo ""
    echo "Files created:"
    echo "- fullchain.pem (certificate + chain)"
    echo "- privkey.pem (private key)"
    echo ""
    echo "Next: Update docker-compose.yml to mount these certificates to Traefik"
else
    echo ""
    echo "❌ Certificate generation failed!"
    echo "Check the error messages above."
fi
