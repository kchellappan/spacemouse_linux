#!/bin/bash

# Check if this is running on Ubuntu 24
lsb_release -sr | grep -q "24."
IS_UBUNTU_24=$?
if [ "${IS_UBUNTU_24}" -eq "0" ]; then
    echo "This is NOT Ubuntu 24. The install script may not work as intended."
fi

# Install deps to build and run the WAMP server
sudo apt update
sudo apt install build-essential pkg-config libssl-dev curl mkcert caddy python3-venv -y
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Stuff to build the tests if needed
python3 -m venv spacemouse
./spacemouse/bin/pip install websocket-client

