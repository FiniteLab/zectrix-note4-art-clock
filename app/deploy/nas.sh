#!/bin/sh
# 从这台电脑把源码同步到 NAS，在 NAS 上编译，并安装/重启时钟服务。
# 用法：app/deploy/nas.sh <设备 IP> [SSH 主机，默认 nas]
set -eu
device=${1:?用法：app/deploy/nas.sh <设备 IP> [SSH 主机]}
host=${2:-nas}
root=$(cd -- "$(dirname -- "$0")/../.." && pwd)
ssh "$host" 'mkdir -p ~/note4-art-clock'
rsync -a --delete --exclude target --exclude .git --exclude '._*' --exclude .DS_Store \
    --exclude build.log --exclude test.log \
    "$root/app" "$root/zectrix-local-push-sdk" "$host:note4-art-clock/"
ssh "$host" "sh ~/note4-art-clock/app/deploy/nas-build.sh '$device'"
