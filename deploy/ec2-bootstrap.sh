#!/bin/bash
# EC2 Ubuntu 24.04 user data. Contains no application or AWS credentials.
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y docker.io docker-compose-v2 docker-buildx python3 curl ca-certificates
systemctl enable --now docker
usermod -aG docker ubuntu
install -d -o ubuntu -g ubuntu -m 755 /opt/sanscue
# Low-cost instance: cap build concurrency and provide encrypted-EBS-backed swap.
if [ ! -f /swapfile ]; then
  fallocate -l 4G /swapfile
  chmod 600 /swapfile
  mkswap /swapfile
  swapon /swapfile
  echo '/swapfile none swap sw 0 0' >> /etc/fstab
fi
cat > /etc/systemd/system/sanscue.service <<'UNIT'
[Unit]
Description=SansCue demo runtime
Requires=docker.service
After=docker.service network-online.target
Wants=network-online.target
ConditionPathExists=/opt/sanscue/deploy/production.sh

[Service]
Type=oneshot
RemainAfterExit=yes
WorkingDirectory=/opt/sanscue
ExecStartPre=/usr/bin/python3 -B deploy/configure-production.py
ExecStart=/bin/sh deploy/production.sh up -d --wait --wait-timeout 180
ExecStop=/bin/sh deploy/production.sh stop
TimeoutStartSec=300
TimeoutStopSec=120

[Install]
WantedBy=multi-user.target
UNIT
systemctl daemon-reload
# Enable only after source, images and acceptance checks are installed.
