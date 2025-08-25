#!/usr/bin/env bash

set -euo pipefail

DISCORD_WEBHOOK_URL="${DISCORD_WEBHOOK_URL:-}"

PRUNE_CONTAINERS="${PRUNE_CONTAINERS:-true}"             
PRUNE_IMAGES="${PRUNE_IMAGES:-true}"                     
PRUNE_VOLUMES="${PRUNE_VOLUMES:-false}"                  
PRUNE_BUILDER_CACHE="${PRUNE_BUILDER_CACHE:-true}"      

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log() {
    echo -e "${GREEN}[$(date '+%Y-%m-%d %H:%M:%S')]${NC} $1" >&2
}

error() {
    echo -e "${RED}[$(date '+%Y-%m-%d %H:%M:%S')] ERROR:${NC} $1" >&2
}

warning() {
    echo -e "${YELLOW}[$(date '+%Y-%m-%d %H:%M:%S')] WARNING:${NC} $1"
}

get_system_usage() {
    local ram_total=$(free -h | awk '/^Mem:/ {print $2}')
    local ram_used=$(free -h | awk '/^Mem:/ {print $3}')
    local ram_percent=$(free | awk '/^Mem:/ {printf "%.1f", $3/$2 * 100.0}')
    
    local disk_usage=$(df -h / | awk 'NR==2 {print $3 "/" $2 " (" $5 ")"}')
    
    local docker_images_count=$(docker images -q | wc -l)
    local docker_containers_count=$(docker ps -a -q | wc -l)
    local docker_volumes_count=$(docker volume ls -q | wc -l)
    
    local docker_usage=$(docker system df --format "table {{.Type}}\t{{.TotalCount}}\t{{.Size}}\t{{.Reclaimable}}" | tail -n +2)
    
    echo "📊 **System Usage Report**

    🖥️ **RAM Usage:** $ram_used / $ram_total ($ram_percent%)
    💾 **Disk Usage:** $disk_usage

    🐳 **Docker Resources:**
    📦 Images: $docker_images_count
    🔲 Containers: $docker_containers_count  
    💿 Volumes: $docker_volumes_count

    📈 **Docker Storage:**
    \`\`\`
    $docker_usage
    \`\`\`"
}

send_discord_notification() {
    local full_message="$1"
    local color="${2:-3447003}"
    
    if [[ -z "$DISCORD_WEBHOOK_URL" ]]; then
        warning "DISCORD_WEBHOOK_URL not set, skipping notification"
        return 0
    fi
    
    local json_payload
    json_payload=$(jq -n \
        --arg msg "$full_message" \
        --arg ts "$(date -u +%Y-%m-%dT%H:%M:%S.000Z)" \
        --argjson color "$color" \
        '{embeds:[{title:"PWS Docker Maintenance", description:$msg, color:$color, timestamp:$ts, footer:{text:"PWS CRON JOB"}}]}')
                
    local curl_result
    curl_result=$(curl -H "Content-Type: application/json" \
        -d "$json_payload" \
        "$DISCORD_WEBHOOK_URL" \
        --silent --show-error --write-out "HTTP_%{http_code}" 2>&1)
    
    if [[ "$curl_result" == *"HTTP_2"* ]] || [[ "$curl_result" == *"HTTP_204"* ]]; then
        log "✅ Discord notification sent successfully"
        return 0
    else
        error "Failed to send Discord notification: $curl_result"
        return 1
    fi
}

safe_docker_prune() {
    log "Starting Docker Pruning"

    # Containers
    local containers_cmd=(docker container prune -f)
    local pruned_containers="Skipped"
    if [[ "$PRUNE_CONTAINERS" == "true" ]]; then
        log "Pruning stopped containers..."
        pruned_containers=$("${containers_cmd[@]}" 2>&1 || true)
    fi

    # Images
    local images_cmd=(docker image prune -f -a)
    local pruned_images="Skipped"
    if [[ "$PRUNE_IMAGES" == "true" ]]; then
        log "Pruning dangling images and unused images..."
        pruned_images=$("${images_cmd[@]}" 2>&1 || true)
    fi

    # Volumes
    local volumes_cmd=(docker volume prune -f)
    local pruned_volumes="Skipped"
    if [[ "$PRUNE_VOLUMES" == "true" ]]; then
        log "Pruning unused volumes..."
        pruned_volumes=$("${volumes_cmd[@]}" 2>&1 || true)
    fi

    # Builder cache
    local builder_cmd=(docker builder prune -f)
    local pruned_cache="Skipped"
    if [[ "$PRUNE_BUILDER_CACHE" == "true" ]]; then
        log "Pruning builder cache..."
        pruned_cache=$("${builder_cmd[@]}" 2>&1 || true)
    fi

    local containers_space=$(echo "$pruned_containers" | grep -o "Total reclaimed space: .*" | head -1 || true)
    local images_space=$(echo "$pruned_images" | grep -o "Total reclaimed space: .*" | head -1 || true)
    local volumes_space=$(echo "$pruned_volumes" | grep -o "Total reclaimed space: .*" | head -1 || true)
    local cache_space=$(echo "$pruned_cache" | grep -o "Total reclaimed space: .*" | head -1 || true)

    local summary="🧹 Prune Results:

    📦 Containers: ${containers_space:-No change}
    🖼️ Images: ${images_space:-No change}
    💾 Volumes: ${volumes_space:-No change}
    🔨 Build Cache: ${cache_space:-No change}"

    echo "$summary"
}

main() {
    log "Starting Docker maintenance routine..."
    
    # Notif Before
    log "Getting system usage before pruning..."
    local usage_before=$(get_system_usage)
    
    log "Sending pre-pruning notification to Discord..."
    send_discord_notification "⏰ Daily Maintenance Started

$usage_before

🔄 Starting cleanup process..." 16776960
    
    local prune_results=$(safe_docker_prune)

    sleep 5
    
    # Notif After
    log "Getting system usage after pruning..."
    local usage_after=$(get_system_usage)
    
    log "Sending post-pruning usage notification to Discord..."
    send_discord_notification "✅ Daily Maintenance Completed

$usage_after" 65280

    log "Sending prune results notification to Discord..."
    send_discord_notification "$prune_results

🎉 Maintenance completed successfully!" 65280
    
    log "Docker maintenance routine completed successfully!"
}

main "$@"