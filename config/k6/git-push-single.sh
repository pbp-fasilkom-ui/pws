#!/usr/bin/env bash
set -euo pipefail

# Single user git push script for k6 load testing
USERNAME=${1:?username required}
GIT_USERNAME=${2:?git username required}
GIT_PASSWORD=${3:?git password required}
PROJECT_NAME=${4:?project name required}

REPO_PATH="clone/$USERNAME"
GIT_URL="http://$GIT_USERNAME:$GIT_PASSWORD@localhost:8080/git/$USERNAME/$PROJECT_NAME.git"

# Logging
LOG_DIR="logs"
mkdir -p "$LOG_DIR"
LOG_FILE="$LOG_DIR/${USERNAME}.log"

# Function untuk capture output dan error
run_git_cmd() {
    local cmd="$1"
    echo ">>> Executing: $cmd"
    echo ">>> Output:"
    
    # Capture both stdout dan stderr, dengan exit code
    set +e
    output=$(eval "$cmd" 2>&1)
    exit_code=$?
    set -e
    
    # Print output (bisa kosong)
    if [ -n "$output" ]; then
        echo "$output"
    else
        echo "(no output)"
    fi
    
    echo ">>> Exit code: $exit_code"
    echo ">>> Command completed"
    echo ""
    
    return $exit_code
}

{
  echo "=== $(date -Is) Git push for $USERNAME ==="
  echo "Username: $USERNAME"
  echo "Git Username: $GIT_USERNAME"
  echo "Git Password: $GIT_PASSWORD"
  echo "Project Name: $PROJECT_NAME"
  echo "Repo: $REPO_PATH"
  echo "Remote: $GIT_URL"
  echo ""

  # Enhanced git environment for maximum verbosity
  export GIT_TERMINAL_PROMPT=0
  export GIT_ASKPASS=echo
  export GIT_SSH_COMMAND="ssh -oBatchMode=yes"
  export GIT_TRACE=1
  export GIT_TRACE_CURL=1
  export GIT_CURL_VERBOSE=1
  export GIT_TRACE_PACKET=1
  export GIT_TRACE_PERFORMANCE=1
  export GIT_TRACE_SETUP=1

  # Check if repo exists
  if [ ! -d "$REPO_PATH/.git" ]; then
      echo "ERROR: Repository $REPO_PATH does not exist or is not a git repo"
      exit 2
  fi

  cd "$REPO_PATH"
  echo "Changed directory to: $(pwd)"
  echo ""

  # Git status dengan full output
  if ! run_git_cmd "git status -sb"; then
      echo "WARNING: Git status failed, continuing anyway"
  fi

  # Configure git user
  echo "Configuring git user..."
  run_git_cmd "git config user.email '${USERNAME}@test.com'"
  run_git_cmd "git config user.name '$USERNAME'"

  # Show current config
  echo "Current git config:"
  run_git_cmd "git config --local --list"

  # Set branch to master
  echo "Setting branch to master..."
  run_git_cmd "git branch -M master" || echo "Branch already exists or command failed, continuing..."

  # Show current branch
  run_git_cmd "git branch -vv"

  # Add all changes dengan detail
  echo "Adding all changes..."
  if ! run_git_cmd "git add -A"; then
      echo "ERROR: Git add failed for $USERNAME"
      exit 3
  fi

  # Show what's staged
  echo "Staged changes:"
  run_git_cmd "git diff --cached --name-status"

  # Commit changes
  echo "Committing changes..."
  commit_msg="Load test push from $USERNAME $(date -Is)"
  if ! run_git_cmd "git commit -m '$commit_msg' --allow-empty"; then
      echo "ERROR: Git commit failed for $USERNAME"
      exit 4
  fi

  # Show commit info
  run_git_cmd "git log --oneline -1"

  # Remote handling dengan detail
  echo "Handling remote configuration..."
  
  if run_git_cmd "git remote get-url origin"; then
    echo "Remote origin exists, updating URL..."
    run_git_cmd "git remote set-url origin '$GIT_URL'"
  else
    echo "Remote origin doesn't exist, adding..."
    run_git_cmd "git remote add origin '$GIT_URL'" || echo "Failed to add remote, continuing..."
  fi

  # Show remote info
  run_git_cmd "git remote -v"

  # Pre-push info
  echo "Repository state before push:"
  run_git_cmd "git log --oneline -3"
  run_git_cmd "git branch -vv"

  # The main push dengan maximum verbosity
  echo "=== EXECUTING GIT PUSH ==="
  echo "Push command: git push origin master"
  echo "Timestamp: $(date -Is)"
  echo ""

  # Capture push output dengan detail maksimal
  set +e
  push_output=$(git push origin master 2>&1)
  push_exit_code=$?
  set -e

  echo "=== PUSH OUTPUT START ==="
  echo "$push_output"
  echo "=== PUSH OUTPUT END ==="
  echo ""
  echo "Push exit code: $push_exit_code"
  echo "Push completed at: $(date -Is)"

  if [ $push_exit_code -eq 0 ]; then
      echo ""
      echo "SUCCESS: Git push successful for $USERNAME"
      
      # Post-push verification
      echo "Post-push repository state:"
      run_git_cmd "git log --oneline -1"
      run_git_cmd "git branch -vv"
      
      exit 0
  else
      echo ""
      echo "ERROR: Git push failed for $USERNAME (exit code: $push_exit_code)"
      
      # Debug info on failure
      echo "Debug information:"
      run_git_cmd "git status -sb"
      run_git_cmd "git remote -v"
      run_git_cmd "git log --oneline -3"
      
      exit 5
  fi

} 2>&1 | tee -a "$LOG_FILE"