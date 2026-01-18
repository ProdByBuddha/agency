#!/bin/bash

# start_otel_safe.sh - Managed, Sandboxed OpenTelemetry Collector
# This script executes the OTel collector within a MacOS Seatbelt (sandbox-exec) container.

BIN_PATH=$(which otelcol || which otelcol-contrib)

if [ -z "$BIN_PATH" ]; then
    echo "❌ OpenTelemetry Collector (otelcol) not found."
    echo "Please install it via Homebrew: brew install open-telemetry-collector"
    exit 1
fi

CONFIG_PATH="./config/otel-config.yaml"
SB_PROFILE="./scripts/otel-collector.sb"

echo "🛡️  Starting OpenTelemetry Collector in Seatbelt Sandbox..."
echo "📍 Binary: $BIN_PATH"
echo "📍 Config: $CONFIG_PATH"

/usr/bin/sandbox-exec -f "$SB_PROFILE" "$BIN_PATH" --config "$CONFIG_PATH"
