#!/bin/bash

# Simple registration script for load testing users
DOMAIN="http://localhost:8080"
USER_CSV="user.csv"

echo "Starting user registration..."
echo "Target domain: $DOMAIN"

# Read users from CSV (skip header)
tail -n +2 "$USER_CSV" | while IFS=',' read -r username password; do
    # Clean username (remove any whitespace)
    username=$(echo "$username" | tr -d ' \r\n')
    password=$(echo "$password" | tr -d ' \r\n')
    
    echo "Registering user: $username"
    
    # Register user via API
    response=$(curl -s -w "%{http_code}" -o /tmp/reg_response.json \
        -X POST "$DOMAIN/api/register" \
        -H "Content-Type: application/json" \
        -d "{\"username\":\"$username\",\"password\":\"$password\",\"email\":\"${username}@loadtest.com\",\"name\":\"$username\"}")
    
    http_code="${response: -3}"
    
    if [ "$http_code" = "201" ] || [ "$http_code" = "200" ]; then
        echo "✓ User $username registered successfully"
    elif [ "$http_code" = "400" ] || [ "$http_code" = "409" ]; then
        echo "- User $username already exists"
    else
        echo "✗ Failed to register $username: $http_code"
        cat /tmp/reg_response.json
    fi
    
    # Small delay to avoid overwhelming
    sleep 0.1
done

rm -f /tmp/reg_response.json
echo "User registration process completed"
