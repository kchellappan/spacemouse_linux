#!/bin/bash

# If there are any Caddy instances running, kill them
sudo pkill -9 cady

# Run Caddy for reverse proxy 
nohup caddy run --config ./Caddyfile &

# Run the Rust WAMP server
cargo run