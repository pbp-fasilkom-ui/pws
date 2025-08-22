#!/bin/bash

# Setup repositories for load testing
BASE_REPO="https://github.com/isaui/repo-django-load-testing.git"
CLONE_DIR="clone"
USER_CSV="user.csv"

echo "Starting repository setup for load testing..."
echo "Base repository: $BASE_REPO"

# Create clone directory
mkdir -p "$CLONE_DIR"

# Read users from CSV (skip header)
tail -n +2 "$USER_CSV" | while IFS=',' read -r username password; do
    # Clean username (remove any whitespace)
    username=$(echo "$username" | tr -d ' \r\n')
    
    PROJECT_DIR="$CLONE_DIR/$username"
    
    echo "Setting up repo for: $username"
    
    # Check if repo already exists
    if [ -d "$PROJECT_DIR" ]; then
        echo "Repository for $username already exists, skipping clone"
    else
        echo "Cloning $BASE_REPO for $username..."
        if git clone "$BASE_REPO" "$PROJECT_DIR"; then
            echo "✓ Successfully cloned repo for $username"
        else
            echo "✗ Failed to clone repo for $username"
            continue
        fi
    fi
    
    # Modify tester/settings.py with unique configuration
    echo "Modifying settings.py for $username..."
    
    TIMESTAMP=$(date +%s)
    RANDOM_SEED=$((RANDOM * RANDOM))
    DEBUG_STR="True"
    
    # Append unique settings to tester/settings.py
    cat >> "$PROJECT_DIR/tester/settings.py" << EOF

# Load Test Configuration for $username
LOAD_TEST_USER = "$username"
LOAD_TEST_TIMESTAMP = $TIMESTAMP
BUILD_ID = "build_${username}_${TIMESTAMP}"
RANDOM_SEED = $RANDOM_SEED
DEBUG = $DEBUG_STR
SECRET_KEY = "django-insecure-${username}-${RANDOM_SEED}"

EOF
    
    if [ $? -eq 0 ]; then
        echo "✓ Successfully modified source code for $username"
    else
        echo "✗ Failed to modify source code for $username"
    fi
    
    # Small delay to avoid overwhelming the system
    sleep 0.1
done

echo "Repository setup process completed"
echo "All repositories prepared for load testing"
