#!/bin/bash

# Install deps to build and run the WAMP server
sudo apt update
sudo apt install build-essential pkg-config libssl-dev curl mkcert caddy -y
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Stuff to build the tests if needed
python3 -m venv spacemouse
./spacemouse/bin/pip install websocket-client

