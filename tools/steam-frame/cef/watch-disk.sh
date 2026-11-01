#!/usr/bin/env bash
# Watches the Mac's system disk while a long CEF build runs and stops the build's container (CEF_CONTAINER, as
# build.sh names it) when the disk's free space falls under the limit: macOS keeps its swap there, and the build
# VM's memory, full for hours, pushes the Mac to swap more. Ends with the container, or when it stops it.
#
#   tools/steam-frame/cef/watch-disk.sh <container> [limit in GB, default 6]
set -uo pipefail
container=$1
limit=${2:-6}
log=${VPI_CEF_WORK:-$(cd "$(dirname "$0")/../../../.." && pwd)/cef}/logs/watch-disk.log
lowest=999999
# the container may take a moment to start
for _ in $(seq 60); do
   docker ps --format '{{.Names}}' | grep -qx "$container" && break
   sleep 5
done
while docker ps --format '{{.Names}}' | grep -qx "$container"; do
   free=$(df -g / | awk 'NR==2 {print $4}')
   swap=$(sysctl -n vm.swapusage | awk '{print $6}')
   (( free < lowest )) && lowest=$free && echo "$(date '+%F %T') system disk: $free GB free; swap used: $swap" >> "$log"
   if (( free < limit )); then
      echo "$(date '+%F %T') system disk at $free GB free, under $limit: stopping $container" | tee -a "$log"
      docker stop "$container" > /dev/null
      exit 1
   fi
   sleep 60
done
echo "$(date '+%F %T') $container ended; the system disk's lowest: $lowest GB free" | tee -a "$log"
